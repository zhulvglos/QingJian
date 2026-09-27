"""长录音按片段处理；模型仅加载一次，每片结果独立持久化。"""
import json
import os
import sys
from pathlib import Path
# 嵌入式 Python 的 _pth 不包含脚本目录，显式加入本次独立任务目录。
sys.path.insert(0, str(Path(__file__).resolve().parent))
from worker import initialize, transcribe


def commit(path, value):
    temporary = path.with_suffix(".tmp")
    with temporary.open("w", encoding="utf-8") as f:
        json.dump(value, f, ensure_ascii=False)
        f.flush()
        os.fsync(f.fileno())
    temporary.replace(path)


try:
    request = json.loads(Path("request.json").read_text(encoding="utf-8"))
    engine = initialize(request)
    Path("results").mkdir(exist_ok=True)
    for index, item in enumerate(request["segments"]):
        try:
            value = {"ok": True, "text": transcribe(item["path"], engine)}
        except Exception as exc:
            value = {"ok": False, "error": type(exc).__name__ + ": " + str(exc)}
        # 主进程仅在原子更名完成后读取，避免将半截 JSON 当作转写失败。
        commit(Path("results") / (str(index) + ".json"), value)
    commit(Path("response.json"), {"ok": True})
except Exception as exc:
    commit(Path("response.json"), {"ok": False, "error": type(exc).__name__ + ": " + str(exc)})
