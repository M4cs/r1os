"""Speeds up the speech-to-text model for the R1 (ADR-0009): int8 weights and ONNX Runtime's
pre-optimized .ort format, so loading skips graph optimization.

Run through tools/fetch-speech.sh, or by hand:
    uv run --with onnxruntime==1.28.0 --with onnx python tools/optimize-stt.py out/speech/stt OUT_DIR [variant]

Variants (written as complete model directories, tokens and all):
    ort       the fp32 encoder/decoder/joiner as .ort
    int8      the encoder quantized to int8 (dynamic, per-channel weights), decoder/joiner fp32
    int8-ort  int8 as .ort

The onnxruntime minor version must match the R1's libonnxruntime.so (1.28.2 in sherpa-onnx 1.13.8;
PyPI has 1.28.0, and patch releases share the format):
.ort files are tied to the runtime that wrote them.
"""

import shutil
import subprocess
import sys
from pathlib import Path

from onnxruntime.quantization import QuantType, quantize_dynamic

KEEP = ["tokens.txt", "MODEL", "test.wav"]
PARTS = ["encoder", "decoder", "joiner"]


def copy_meta(src: Path, dst: Path, suffix: str) -> None:
    dst.mkdir(parents=True, exist_ok=True)
    for f in KEEP:
        if (src / f).exists():
            shutil.copy2(src / f, dst / f)
    model = (src / "MODEL").read_text().strip() if (src / "MODEL").exists() else src.name
    (dst / "MODEL").write_text(f"{model} ({suffix})\n")


def to_ort(onnx_dir: Path, names: list[str]) -> None:
    # Fixed style: optimizations are baked in for an ARM CPU, so the R1 doesn't redo them at load.
    for n in names:
        subprocess.run(
            [sys.executable, "-m", "onnxruntime.tools.convert_onnx_models_to_ort", str(onnx_dir / n),
             "--output_dir", str(onnx_dir), "--optimization_style", "Fixed", "--target_platform", "arm"],
            check=True,
        )
        (onnx_dir / n).unlink()  # keep only the .ort, so r1d can't pick the wrong one


def int8(src: Path, dst: Path) -> None:
    quantize_dynamic(str(src / "encoder.onnx"), str(dst / "encoder.int8.onnx"), weight_type=QuantType.QInt8, per_channel=True)
    for p in ["decoder", "joiner"]:
        shutil.copy2(src / f"{p}.onnx", dst / f"{p}.onnx")


def main() -> None:
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    variants = sys.argv[3:] or ["ort"]
    for v in variants:
        dst = out / v
        shutil.rmtree(dst, ignore_errors=True)
        copy_meta(src, dst, v)
        if v == "ort":
            # Models ship either plain or already-int8 files (encoder.int8.onnx); keep the name.
            names = [f"{p}.int8.onnx" if (src / f"{p}.int8.onnx").exists() else f"{p}.onnx" for p in PARTS]
            for n in names:
                shutil.copy2(src / n, dst / n)
            to_ort(dst, names)
        elif v == "int8":
            int8(src, dst)
        elif v == "int8-ort":
            int8(src, dst)
            to_ort(dst, ["encoder.int8.onnx", "decoder.onnx", "joiner.onnx"])
        else:
            sys.exit(f"unknown variant {v}")
        size = sum(f.stat().st_size for f in dst.iterdir()) / 1e6
        print(f"{v}: {dst} ({size:.0f} MB): {sorted(f.name for f in dst.iterdir())}")


if __name__ == "__main__":
    main()
