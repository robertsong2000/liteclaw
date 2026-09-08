#!/usr/bin/env python3
"""manual-rag single entry point.

Usage:
  rag.py "<question>"    Hybrid search (BM25 + vector) over the manual index.
  rag.py --ingest        (Re)build the index from /workspace/manual/.

Self-contained script: importable helpers allow both modes in one file.
"""

import json
import math
import os
import re
import subprocess
import sys
import time
import urllib.request

MANUAL_DIR = "/workspace/manual"
INDEX_DIR = os.path.join(MANUAL_DIR, ".index")
INDEX_FILE = os.path.join(INDEX_DIR, "index.json")
OLLAMA = os.environ.get("OLLAMA_URL", "http://172.21.0.1:11434")
EMBED_MODEL = os.environ.get("EMBED_MODEL", "bge-m3")
MAX_CHARS = 700  # the embedding runner has a hard 512-token physical batch
MAX_EMBED_CHARS = MAX_CHARS
RRF_K = 60
TOKEN_RE = re.compile(r"[\w]+", re.UNICODE)

# ---------------------------------------------------------------- embedding


def _embed_one(text):
    """Embed a single text via the legacy /api/embeddings endpoint.

    The newer /api/embed endpoint on this Ollama build returns corrupted
    vectors (verified 2026-09: related-text cos ~0 while legacy gives ~0.8),
    so we deliberately use the legacy one and validate every result.
    """
    req = urllib.request.Request(
        f"{OLLAMA}/api/embeddings",
        data=json.dumps({
            "model": EMBED_MODEL,
            "prompt": text,
            # slot defaults to 512 ctx on this build and rejects long chunks
            "options": {"num_ctx": 8192},
        }).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=300) as resp:
        vec = json.load(resp)["embedding"]
    if len(vec) == 0 or not all(math.isfinite(x) for x in vec) or sum(x * x for x in vec) < 1e-12:
        raise ValueError("invalid embedding returned")
    return vec


def embed(texts):
    vecs = []
    for i, text in enumerate(texts):
        for attempt in range(5):
            try:
                vecs.append(_embed_one(text))
                break
            except Exception as e:
                body = ""
                if hasattr(e, "read"):
                    try:
                        body = e.read().decode()[:200]
                    except Exception:
                        pass
                if attempt == 4:
                    raise
                print(f"warn: embed retry {attempt + 1} for chunk {i}: {e} {body}", file=sys.stderr)
                time.sleep(5)
        if (i + 1) % 50 == 0:
            print(f"embedded {i + 1}/{len(texts)}", file=sys.stderr)
    return vecs


def self_check(records, sample=5, threshold=0.95):
    """Re-embed a few stored chunks; vectors must match or the index is bad."""
    import random
    picks = random.sample(range(len(records)), min(sample, len(records)))
    for i in picks:
        vec = _embed_one(records[i]["text"])
        dot = sum(x * y for x, y in zip(vec, records[i]["embedding"]))
        norm = math.sqrt(sum(x * x for x in vec)) * math.sqrt(
            sum(x * x for x in records[i]["embedding"])
        )
        if dot / norm < threshold:
            raise ValueError(
                f"self-check failed for chunk {i}: index vectors are corrupt, re-run ingest"
            )
    print(f"self-check passed on {len(picks)} sampled chunks", file=sys.stderr)


# ---------------------------------------------------------------- ingest


def clean(text):
    """Light cleanup of PDF-extraction artifacts."""
    text = re.sub(r"(\w)-\s*\n\s*(\w)", r"\1\2", text)  # join hyphenated line breaks
    text = re.sub(r"\s*\n\s*", " ", text)  # collapse newlines/indentation
    return text.strip()


def split_long(text):
    """Split overlong text at sentence boundaries; chunks must fit any slot."""
    if len(text) <= MAX_EMBED_CHARS:
        return [text]
    sentences = re.split(r"(?<=[.!?]) +", text)
    parts, buf = [], ""
    for s in sentences:
        if buf and len(buf) + len(s) + 1 > MAX_EMBED_CHARS:
            parts.append(buf)
            buf = s
        else:
            buf = f"{buf} {s}" if buf else s
    if buf:
        parts.append(buf)
    return parts


def read_jsonl(path):
    """Pre-chunked records with metadata; content is cleaned but not re-chunked."""
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            r = json.loads(line)
            content = clean(r.get("content", ""))
            for part_i, part in enumerate(split_long(content)):
                if len(part) < 20:
                    continue
                cid = r.get("chunk_id")
                if len(split_long(content)) > 1:
                    cid = f"{cid}#{part_i + 1}"
                out.append({
                    "chunk_id": cid,
                    "title": r.get("title"),
                    "section": r.get("section_path") or r.get("title"),
                    "source": f"{r.get('source_file') or os.path.basename(path)} p.{r.get('page_start')}-{r.get('page_end')}",
                    "safety_level": r.get("safety_level"),
                    "text": part,
                })
    return out


def read_pdf(path):
    # -layout keeps tables readable enough for chunking.
    out = subprocess.run(
        ["pdftotext", "-layout", path, "-"], capture_output=True, text=True
    )
    if out.returncode != 0:
        print(f"warn: pdftotext failed on {path}: {out.stderr.strip()}", file=sys.stderr)
        return []
    return [out.stdout]


def read_plain(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return [f.read()]


def split_sections(text, ext):
    """Split into (section_title, text) pairs at markdown headings."""
    if ext != ".md":
        return [("", text)]
    sections = []
    cur_title, buf = "", []
    for line in text.splitlines():
        if re.match(r"#{1,4} ", line):
            if buf:
                sections.append((cur_title, "\n".join(buf)))
            cur_title, buf = line.lstrip("#").strip(), [line]
        else:
            buf.append(line)
    if buf:
        sections.append((cur_title, "\n".join(buf)))
    return sections


def make_chunks(text):
    """Split a section into paragraph groups up to MAX_CHARS."""
    paras = [p.strip() for p in re.split(r"\n\s*\n", text) if p.strip()]
    chunks, buf = [], ""
    for p in paras:
        if buf and len(buf) + len(p) + 2 > MAX_CHARS:
            chunks.append(buf)
            buf = p
        else:
            buf = f"{buf}\n\n{p}" if buf else p
    if buf:
        chunks.append(buf)
    return chunks


def cmd_ingest():
    files = []
    for root, _, names in os.walk(MANUAL_DIR):
        if ".index" in root:
            continue
        for n in sorted(names):
            ext = os.path.splitext(n)[1].lower()
            if ext in (".md", ".txt", ".pdf", ".jsonl"):
                files.append((os.path.join(root, n), ext))

    records, batch = [], []
    for path, ext in files:
        rel = os.path.relpath(path, MANUAL_DIR)
        if ext == ".jsonl":
            for rec in read_jsonl(path):
                records.append(rec)
                batch.append(rec["text"])
            continue
        texts = read_pdf(path) if ext == ".pdf" else read_plain(path)
        for raw in texts:
            for title, section in split_sections(raw, ext):
                for chunk in make_chunks(section):
                    if len(chunk) < 20:
                        continue
                    records.append({"source": rel, "section": title, "text": chunk})
                    batch.append(chunk)

    if not records:
        print(f"no documents found under {MANUAL_DIR}", file=sys.stderr)
        return 1

    vecs = embed(batch)
    for rec, vec in zip(records, vecs):
        rec["embedding"] = vec

    self_check(records)

    os.makedirs(INDEX_DIR, exist_ok=True)
    with open(INDEX_FILE, "w", encoding="utf-8") as f:
        json.dump(records, f, ensure_ascii=False)
    print(f"indexed {len(records)} chunks from {len(files)} files -> {INDEX_FILE}")
    return 0


# ---------------------------------------------------------------- search


def tokens(text):
    return [t.lower() for t in TOKEN_RE.findall(text)]

# zh->en domain glossary: the corpus is English, BM25 gets zero signal from
# Chinese queries and the vector path alone under-ranks on some sections.
# Append matched English terms so both retrieval paths get usable signal.
ZH_GLOSSARY = {
    # 灯光
    "雾灯": "fog light",
    "大灯": "headlight dipped beam",
    "远光": "main beam headlight",
    "转向灯": "indicator",
    "刹车灯": "stop light brake light",
    "倒车灯": "reversing light",
    "日行灯": "daytime running lights",
    "警告灯": "warning lamp",
    "仪表盘": "instrument panel dashboard",
    # 轮胎
    "胎压": "tyre pressure",
    "轮胎": "tyre",
    "防滑链": "snow chains",
    "备胎": "spare wheel",
    # 充电与电动
    "充电": "charging charge",
    "充电口": "charging socket flap",
    "快充": "rapid DC charging",
    "慢充": "AC charging",
    "电池": "battery",
    "续航": "range",
    "能量回收": "regenerative braking",
    "对外放电": "V2L",
    "高压": "high voltage",
    # 驾驶辅助
    "巡航": "cruise control",
    "自适应巡航": "adaptive cruise control",
    "车道": "lane keeping assistance",
    "盲区": "blind spot warning",
    "雷达": "parking sensors",
    "倒车影像": "reversing camera",
    "自动泊车": "automated parking",
    "紧急制动": "emergency brake assistance",
    # 安全
    "安全气囊": "airbag",
    "安全带": "seat belt",
    "童锁": "child lock",
    "防盗": "immobiliser alarm",
    # 舒适与车身
    "空调": "air conditioning",
    "座椅加热": "heated seat",
    "后视镜": "mirror",
    "车窗": "window",
    "天窗": "sunroof",
    "雨刷": "wiper",
    "雨刮": "wiper",
    "除雾": "demisting defrosting",
    "方向盘": "steering wheel",
    "后备箱": "boot luggage compartment",
    "拖车": "towing trailer",
    "车顶架": "roof rack",
    "门锁": "central locking",
    # 维护
    "保养": "maintenance service",
    "玻璃水": "washer fluid",
    "清洗液": "washer fluid",
    "冷却液": "coolant",
    "制动液": "brake fluid",
    # 钥匙与多媒体
    "钥匙": "key card",
    "导航": "navigation",
    "多媒体": "multimedia",
    "蓝牙": "bluetooth",
    # 其他
    "洗车": "car wash",
    "拖车绳": "towing",
    "驻车": "parking brake",
    "手刹": "parking brake",
    "挡位": "gear control",
    "换挡": "gear control",
    "限速": "speed limiter",
    "故障": "operating faults",
    "启动": "starting stopping the engine",
    "熄火": "stopping the engine",
    "紧急呼叫": "emergency call",
    "SOS": "emergency call",
    "装载": "transporting objects",
    "运输": "transporting objects",
    "保养记录": "service sheets",
    "座椅": "seat",
    "驾驶位": "driving position",
    "驾驶模式": "MULTI-SENSE",
    "后视": "rear view camera",
    "抛锚": "breakdown recovery",
    "救援": "breakdown recovery",
    "储物": "passenger compartment storage",
    "车内装备": "passenger compartment equipment",
    "前舱": "engine compartment",
    "防腐": "anticorrosion check",
    "经济驾驶": "eco-driving",
    "省电": "eco-driving",
    "加装": "accessories",
    "附件": "accessories",
    "车架号": "VIN",
    "识别码": "VIN",
}

def augment_query(query):
    extra = [en for zh, en in ZH_GLOSSARY.items() if zh in query]
    return query + (" " + " ".join(extra) if extra else "")



def bm25_scores(query, docs):
    """Okapi BM25 over the pre-tokenized docs. docs: list[list[str]]."""
    k1, b, n = 1.5, 0.75, len(docs)
    avgdl = sum(len(d) for d in docs) / max(n, 1)
    df = {}
    for d in docs:
        for term in set(d):
            df[term] = df.get(term, 0) + 1
    q_terms = tokens(query)
    scores = []
    for d in docs:
        score, tf = 0.0, {}
        for t in d:
            tf[t] = tf.get(t, 0) + 1
        for t in q_terms:
            if t not in tf:
                continue
            idf = math.log((n - df[t] + 0.5) / (df[t] + 0.5) + 1)
            score += idf * tf[t] * (k1 + 1) / (tf[t] + k1 * (1 - b + b * len(d) / avgdl))
        scores.append(score)
    return scores


def cosine(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    return dot / (na * nb) if na and nb else 0.0


def embed_query(query):
    return _embed_one(query)


def rrf(rankings, top_k):
    """rankings: list of doc-index lists ordered best-first."""
    scores = {}
    for ranking in rankings:
        for rank, idx in enumerate(ranking):
            scores[idx] = scores.get(idx, 0) + 1 / (RRF_K + rank + 1)
    return sorted(scores, key=scores.get, reverse=True)[:top_k]


def cmd_search(query, top_k):
    query = augment_query(query)
    with open(INDEX_FILE, encoding="utf-8") as f:
        records = json.load(f)
    if not records:
        print("[]", file=sys.stderr)
        return 1

    doc_tokens = [tokens(r["text"]) for r in records]
    bm = bm25_scores(query, doc_tokens)
    bm_rank = [i for i, _ in sorted(enumerate(bm), key=lambda x: x[1], reverse=True) if bm[i] > 0]

    qvec = embed_query(query)
    sims = [cosine(qvec, r["embedding"]) for r in records]
    vec_rank = [i for i, _ in sorted(enumerate(sims), key=lambda x: x[1], reverse=True) if sims[i] > 0.3]

    fused = rrf([bm_rank, vec_rank], top_k)
    hits = [
        {
            "rank": rank + 1,
            "source": records[i].get("source"),
            "section": records[i].get("section"),
            "chunk_id": records[i].get("chunk_id"),
            "safety_level": records[i].get("safety_level"),
            "text": records[i]["text"],
        }
        for rank, i in enumerate(fused)
    ]
    print(json.dumps(hits, ensure_ascii=False, indent=2))
    return 0


# ---------------------------------------------------------------- main


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    if argv[0] in ("ingest", "--ingest", "-i"):
        return cmd_ingest()
    top_k = 6
    query = argv
    if len(argv) >= 2 and argv[0] == "-k":
        top_k = int(argv[1])
        query = argv[2:]
    return cmd_search(" ".join(query), top_k)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
