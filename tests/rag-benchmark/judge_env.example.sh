# judge.py 远程评审配置模板（复制为 ~/judge_env.sh 并填入真实值，chmod 600）
# 用法：source ~/judge_env.sh && python3 judge.py runs/<目录> --vs baseline

# OpenAI 兼容评审端点（不设则评审走本地 ollama 的 JUDGE_MODEL）
export JUDGE_OPENAI_BASE="http://<your-4090-host>:16019/v1"
export JUDGE_OPENAI_KEY="sk-xxxxxxxxxxxxxxxxxxxxxxxx"
export JUDGE_MODEL="qwen38-local"

# 并发评审路数（远程端点建议 3~4）
export JUDGE_CONCURRENCY=4
