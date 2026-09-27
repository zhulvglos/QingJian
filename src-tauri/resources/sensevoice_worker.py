"""轻笺离线适配层；输入输出为独立任务目录中的 JSON，不改写用户模型。"""
import json
import os
from pathlib import Path
import socket
import sys
import wave
import zipfile


def initialize(request):
    sys.dont_write_bytecode = True
    packages = Path(request["runtime"]) / "Lib" / "site-packages"
    if not packages.is_dir():
        raise RuntimeError("运行环境缺少 Lib/site-packages")
    sys.path.insert(0, str(packages))
    # 本地引擎不联网；所有库缓存均写入本次任务目录。
    for name in ["HF_HOME", "MODELSCOPE_CACHE", "NUMBA_CACHE_DIR", "MPLCONFIGDIR", "TORCH_HOME", "XDG_CACHE_HOME"]:
        os.environ[name] = str(Path.cwd() / "cache")
    os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", MODELSCOPE_OFFLINE="1")
    def offline(*args, **kwargs):
        raise RuntimeError("本地转写禁止联网；请检查本地模型和依赖是否完整")
    socket.socket.connect = offline
    socket.create_connection = offline

    # 必要配置复制进任务目录，权重由宿主建立只读使用的硬链接。
    for model, names in {
        "SenseVoiceSmall": ["model.pt", "config.yaml", "configuration.json", "am.mvn", "chn_jpn_yue_eng_ko_spectok.bpe.model"],
        "fsmn-vad": ["model.pt", "config.yaml", "configuration.json", "am.mvn"],
    }.items():
        for name in names:
            path = Path("models") / model / name
            if not path.is_file() or not path.stat().st_size:
                raise RuntimeError("缺少模型文件：" + model + "/" + name)
        if request.get("check"):
            with zipfile.ZipFile(Path("models") / model / "model.pt") as archive:
                if archive.testzip():
                    raise RuntimeError("模型权重完整性检查失败：" + model)

    if request.get("check"):
        Path("integrity.ok").write_text("ok", encoding="ascii")

    import numpy as np
    import torch
    import torchaudio
    from funasr import AutoModel
    from funasr.utils.postprocess_utils import rich_transcription_postprocess
    from opencc import OpenCC

    # 仅记录实际解释器、依赖和缓存来源，用于隔离验收；不包含音频或转写正文。
    import site
    import funasr, opencc
    Path("runtime-origin.json").write_text(json.dumps({
        "python": sys.executable, "prefix": sys.prefix, "sysPath": sys.path,
        "userSiteEnabled": site.ENABLE_USER_SITE,
        "modules": {m.__name__: m.__file__ for m in [np, torch, torchaudio, funasr, opencc]},
        "models": str((Path.cwd() / "models").resolve()),
        "cache": {name: os.environ.get(name) for name in ["HF_HOME", "MODELSCOPE_CACHE", "TORCH_HOME", "XDG_CACHE_HOME"]},
    }, ensure_ascii=False, indent=2), encoding="utf-8")

    torch.set_num_threads(4)
    model = AutoModel(model="models/SenseVoiceSmall", vad_model="models/fsmn-vad",
                      device="cpu", disable_update=True, disable_pbar=True,
                      trust_remote_code=False, ncpu=4,
                      vad_kwargs={"max_single_segment_time": 15000})
    return model, OpenCC("t2s"), np, torch, torchaudio


def transcribe(path, engine):
    model, converter, np, torch, torchaudio = engine
    from funasr.utils.postprocess_utils import rich_transcription_postprocess
    # 录音源统一为 16 kHz 单声道 PCM；直接读 WAV，避免 FFmpeg 与中文参数路径依赖。
    with wave.open(str(path), "rb") as wav:
        if wav.getnchannels() != 1 or wav.getsampwidth() != 2:
            raise RuntimeError("需要单声道 16 位 WAV")
        rate = wav.getframerate()
        samples = np.frombuffer(wav.readframes(wav.getnframes()), dtype="<i2").astype(np.float32) / 32768.0
    if rate != 16000:
        samples = torchaudio.functional.resample(torch.from_numpy(samples), rate, 16000).numpy()
    result = model.generate(input=samples, cache={}, language="auto", use_itn=True,
                            batch_size_s=60, merge_vad=False)
    texts = [converter.convert(rich_transcription_postprocess(item.get("text", ""))).strip() for item in result]
    return "\n".join(t for t in texts if t)


def main():
    request = json.loads(Path("request.json").read_text(encoding="utf-8"))
    engine = initialize(request)
    return {"ok": True, "text": transcribe("input.wav", engine), "integrity": bool(request.get("check"))}


if __name__ == "__main__":
    try:
        response = main()
    except Exception as exc:
        response = {"ok": False, "error": type(exc).__name__ + ": " + str(exc)}
    Path("response.json").write_text(json.dumps(response, ensure_ascii=False), encoding="utf-8")
