#!/usr/bin/env python3
"""manual-rag single entry point.

Usage:
  rag.py "<question>"    Hybrid search (BM25 + vector) over the manual index.
  rag.py --ingest        (Re)build the index from /workspace/manual/.

Self-contained script: importable helpers allow both modes in one file.

Index layout under MANUAL_DIR/.index/ (all written by --ingest):
  vectors.f32  raw float32 embedding matrix, row i = chunk i (row-major)
  meta.json    chunk metadata + text, NO embeddings (small, fast to parse)
  bm25.pkl     precomputed BM25 state: tf maps, doc lengths, df, avgdl,
               plus row norms and the embedding dim

A legacy single-file index.json (embeddings inline, ~20MB) is migrated to
the 3-file layout automatically on first search; --ingest only ever writes
the new layout.
"""

import array
import json
import math
import os
import pickle
import re
import subprocess
import sys
import time
import urllib.request
from operator import mul

try:  # optional: matrix multiply when numpy is installed, stdlib fallback otherwise
    import numpy as np
except ImportError:
    np = None

from pathlib import Path

# Deployment settings stay outside the repository, including credentials.
_config_path = Path.home() / ".liteclaw" / "manual-rag.json"
_config = json.loads(_config_path.read_text()) if _config_path.exists() else {}
MANUAL_DIR = os.environ.get("MANUAL_DIR", _config.get("manual_dir", "/workspace/manual"))
EMBED_BASE_URL = os.environ.get("EMBED_BASE_URL", _config.get("base_url", ""))
EMBED_API_KEY = os.environ.get("EMBED_API_KEY", _config.get("api_key", ""))
OLLAMA = os.environ.get("OLLAMA_URL", "http://172.21.0.1:11434")
EMBED_MODEL = os.environ.get("EMBED_MODEL", _config.get("model", "bge-m3"))
MAX_CHARS = 700  # the embedding runner has a hard 512-token physical batch
MAX_EMBED_CHARS = MAX_CHARS
RRF_K = 60
TOKEN_RE = re.compile(r"[\w]+", re.UNICODE)

# Index files are derived from MANUAL_DIR at call time (not import time) so
# test harnesses can repoint MANUAL_DIR on the imported module.


def _paths():
    d = os.environ.get("MANUAL_INDEX_DIR", _config.get("index_dir", os.path.join(MANUAL_DIR, ".index")))
    return {
        "vectors": os.path.join(d, "vectors.f32"),
        "meta": os.path.join(d, "meta.json"),
        "bm25": os.path.join(d, "bm25.pkl"),
        "legacy": os.path.join(d, "index.json"),
    }


# ---------------------------------------------------------------- embedding


def _embed_one(text):
    """Embed a single text via the legacy /api/embeddings endpoint.

    The newer /api/embed endpoint on this Ollama build returns corrupted
    vectors (verified 2026-09: related-text cos ~0 while legacy gives ~0.8),
    so we deliberately use the legacy one and validate every result.
    """
    if EMBED_BASE_URL:
        req = urllib.request.Request(
            EMBED_BASE_URL.rstrip("/") + "/embeddings",
            data=json.dumps({"model": EMBED_MODEL, "input": text}).encode(),
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + EMBED_API_KEY},
        )
        with urllib.request.urlopen(req, timeout=60) as resp:
            vec = json.load(resp)["data"][0]["embedding"]
        if not vec or not all(math.isfinite(x) for x in vec) or sum(x*x for x in vec) < 1e-12:
            raise ValueError("invalid embedding returned")
        return vec
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


def _atomic_write(path, data_bytes):
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data_bytes)
    os.replace(tmp, path)


def write_index(records):
    """Serialize records into the 3-file index layout (atomic per file)."""
    p = _paths()
    os.makedirs(os.path.dirname(p["vectors"]), exist_ok=True)
    dim = len(records[0]["embedding"])
    flat = array.array("f", (float(x) for r in records for x in r["embedding"]))
    _atomic_write(p["vectors"], flat.tobytes())

    meta = [{k: v for k, v in r.items() if k != "embedding"} for r in records]
    _atomic_write(p["meta"], json.dumps(meta, ensure_ascii=False).encode("utf-8"))

    tfs, doclen, norms = [], [], []
    df = {}
    for r in records:
        toks = tokens(r["text"])
        tf = {}
        for t in toks:
            tf[t] = tf.get(t, 0) + 1
        tfs.append(tf)
        doclen.append(len(toks))
        for t in tf:
            df[t] = df.get(t, 0) + 1
    n = len(records)
    for i in range(n):
        row = flat[i * dim:(i + 1) * dim]
        nrm = math.sqrt(sum(map(mul, row, row)))
        norms.append(nrm if nrm > 0 else 1.0)  # zero rows score 0 either way
    ix = {
        "dim": dim,
        "avgdl": sum(doclen) / max(n, 1),
        "doclen": doclen,
        "df": df,
        "tfs": tfs,
        "norms": norms,
    }
    _atomic_write(p["bm25"], pickle.dumps(ix, protocol=4))


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

    write_index(records)
    self_check()

    p = _paths()
    print(f"indexed {len(records)} chunks from {len(files)} files -> "
          f"{p['vectors']}, {p['meta']}, {p['bm25']}")
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
    "质保": "warranty",
    "保修": "warranty",
    "质保期": "warranty period",
}


def augment_query(query):
    extra = [en for zh, en in ZH_GLOSSARY.items() if zh in query]
    return query + (" " + " ".join(extra) if extra else "")


def bm25_scores(query, ix):
    """Okapi BM25 over pre-tokenized docs (tf maps, df, avgdl from the index)."""
    k1, b = 1.5, 0.75
    tfs, doclen, df, avgdl = ix["tfs"], ix["doclen"], ix["df"], ix["avgdl"]
    n = len(tfs)
    q_terms = tokens(query)
    scores = []
    for i in range(n):
        tf, dl = tfs[i], doclen[i]
        score = 0.0
        for t in q_terms:
            f = tf.get(t)
            if not f:
                continue
            idf = math.log((n - df[t] + 0.5) / (df[t] + 0.5) + 1)
            score += idf * f * (k1 + 1) / (f + k1 * (1 - b + b * dl / avgdl))
        scores.append(score)
    return scores


def similarities(qvec, vecs, ix):
    """Cosine of qvec against every index row, using precomputed row norms."""
    n, dim, norms = len(ix["doclen"]), ix["dim"], ix["norms"]
    if np is not None:
        mat = np.frombuffer(memoryview(vecs), dtype=np.float32).reshape(n, dim)
        q = np.asarray(qvec, dtype=np.float32)
        qn = float(np.linalg.norm(q))
        return (mat @ q / (qn * np.asarray(norms, dtype=np.float32))).tolist()
    q = array.array("f", qvec)
    qn = math.sqrt(sum(map(mul, q, q)))
    out, base = [], 0
    for i in range(n):
        row = vecs[base:base + dim]
        base += dim
        out.append(sum(map(mul, q, row)) / (qn * norms[i]))
    return out


def embed_query(query):
    return _embed_one(query)


def rrf(rankings, top_k):
    """rankings: list of doc-index lists ordered best-first."""
    scores = {}
    for ranking in rankings:
        for rank, idx in enumerate(ranking):
            scores[idx] = scores.get(idx, 0) + 1 / (RRF_K + rank + 1)
    return sorted(scores, key=scores.get, reverse=True)[:top_k]


def load_index():
    """Load the 3-file index; migrate the legacy single JSON on first use.

    Returns (records, vecs, ix) or None when no index exists at all.
    """
    p = _paths()
    if not all(os.path.exists(p[k]) for k in ("vectors", "meta", "bm25")):
        if not os.path.exists(p["legacy"]):
            return None
        print("migrating legacy index.json -> vectors.f32/meta.json/bm25.pkl ...",
              file=sys.stderr)
        t = time.time()
        with open(p["legacy"], encoding="utf-8") as f:
            records = json.load(f)
        write_index(records)
        print(f"migration done in {time.time() - t:.1f}s", file=sys.stderr)
    with open(p["meta"], encoding="utf-8") as f:
        records = json.load(f)
    with open(p["bm25"], "rb") as f:
        ix = pickle.load(f)
    vecs = array.array("f")
    expected = len(records) * ix["dim"]
    try:
        with open(p["vectors"], "rb") as f:
            vecs.fromfile(f, expected)
    except EOFError:
        raise SystemExit("vectors.f32 is truncated — re-run: rag.py --ingest")
    if len(vecs) != expected:
        raise SystemExit("vectors.f32 size mismatch — re-run: rag.py --ingest")
    return records, vecs, ix


def cmd_search(query, top_k):
    original_query = query
    query = augment_query(query)
    loaded = load_index()
    if loaded is None:
        print("[]", file=sys.stderr)
        return 1
    records, vecs, ix = loaded

    bm = bm25_scores(query, ix)
    bm_rank = [i for i, _ in sorted(enumerate(bm), key=lambda x: x[1], reverse=True) if bm[i] > 0]

    qvec = embed_query(query)
    sims = similarities(qvec, vecs, ix)
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
    catalog_path = Path(os.environ.get("MANUAL_IMAGE_DIR", str(Path.home() / ".liteclaw/manual-images"))) / "catalog.json"
    catalog = json.loads(catalog_path.read_text()) if catalog_path.exists() else []
    # Prefer illustrations to full-page context; explicit curated associations
    # also cover diagrams on a different page from their explanatory table.
    catalog.sort(key=lambda img: (not bool(img.get("retrieval_keywords")), img.get("kind") == "source_page"))
    query_words = {word.rstrip('s') for word in re.findall(r'[a-z]{4,}', query.lower())}
    # Curated multilingual keywords also identify the equivalent PDF heading.
    for img in catalog:
        if any(k.lower() in query.lower() for k in img.get('retrieval_keywords', [])):
            query_words.update(word.rstrip('s') for word in re.findall(r'[a-z]{4,}', img.get('page_title', '').lower()))
    explicit_topic = any(any(k.lower() in query.lower() for k in img.get('retrieval_keywords', [])) for img in catalog)
    for hit, record_index in zip(hits, fused):
        pages = re.search(r" p\.(\d+)-(\d+)$", hit.get("source", ""))
        hit["images"] = []
        # Text retrieval can intentionally return weak candidates for the model
        # to reject. Do not automatically illustrate those uncertain results.
        if pages and (explicit_topic or sims[record_index] >= 0.55):
            candidates = []
            first, last = map(int, pages.groups())
            for img in catalog:
                chunk_id = (hit.get("chunk_id") or "").split("#")[0]
                chunk_link = chunk_id in img.get("chunk_ids", [])
                same_source = hit["source"].startswith(img["source_file"] + " p.") or chunk_link
                linked = any(k.lower() in query.lower() for k in img.get("retrieval_keywords", []))
                if same_source and (chunk_link or first-1 <= img["pdf_page"] <= last+1 or linked):
                    title_words = {word.rstrip('s') for word in re.findall(r'[a-z]{4,}', img.get('page_title', '').lower())}
                    topic_match = bool(query_words & title_words) or linked
                    candidates.append((img, 2 if linked else int(topic_match)))
            # A chunk may cross a section boundary. When page headings identify
            # the requested topic, omit neighboring-topic figures from that chunk.
            has_topic_match = any(match for _, match in candidates)
            hit['images'] = [
                {'id': img['id'], 'pdf_page': img['pdf_page'], 'caption': img['alt_text'], 'priority': match}
                for img, match in candidates if match or not has_topic_match
            ]
    # Page links generate candidates, not a ready-to-display gallery. Select
    # by the original question and each figure's local explanation first.
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'lib'))
    from image_select import select_images, shortlist
    ids = list(dict.fromkeys(i['id'] for hit in hits for i in hit['images']))
    by_id = {i['id']: i for i in catalog}
    candidates = shortlist(query, [by_id[i] for i in ids if i in by_id])
    # Select from grounded candidate descriptions using the configured model.
    selected = select_images(original_query, candidates)
    attachments = [{
        'id':i['id'], 'pdf_page':i['pdf_page'], 'caption':i['alt_text'], 'kind':i['kind'],
        'context':i.get('context', ''),
        'role':i['role'], 'priority':2,
    } for i in selected]
    # Whole pages are optional provenance, not extra answer illustrations.
    pages = {(i['source_file'], i['pdf_page']) for i in selected}
    selected_ids = {i['id'] for i in selected}
    references = [i for i in catalog if i['kind'] == 'source_page'
                  and (i['source_file'], i['pdf_page']) in pages and i['id'] not in selected_ids]
    attachments += [{'id':i['id'], 'pdf_page':i['pdf_page'], 'caption':i['alt_text'],
                     'role':'reference', 'priority':0} for i in references[:3]]
    for hit in hits:
        hit['images'] = []
    if hits:
        hits[0]['images'] = attachments
    print(json.dumps(hits, ensure_ascii=False, indent=2))
    return 0


def self_check(sample=5, threshold=0.95):
    """Re-embed a few stored chunks; vectors must match or the index is bad."""
    import random
    loaded = load_index()
    if loaded is None:
        raise ValueError("no index to self-check")
    records, vecs, ix = loaded
    picks = random.sample(range(len(records)), min(sample, len(records)))
    for i in picks:
        vec = _embed_one(records[i]["text"])
        sims = similarities(vec, vecs, ix)
        if sims[i] < threshold:
            raise ValueError(
                f"self-check failed for chunk {i}: index vectors are corrupt, re-run ingest"
            )
    print(f"self-check passed on {len(picks)} sampled chunks", file=sys.stderr)


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
