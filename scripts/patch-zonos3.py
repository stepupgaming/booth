"""Patches a ZONOS3-Mini checkout so Booth's engine builds compile.

    python scripts/patch-zonos3.py <zonos3 checkout> [--msvc] [--backend-dl]

--msvc        MSVC rejects a nested struct that shares its name with a member (C2380). Fixed
              upstream in 43bac93 (probe_head); skipped when the source already has it.
--backend-dl  For GGML_BACKEND_DL builds (the portable CPU engines for Windows and Linux): the CPU
              backend becomes a plugin library, so main() has to load the backends, and the one CPU
              helper the encoders call directly is forwarded through the backend registry.
              Not for static builds (macOS), where the real helper is linked in.
"""
import pathlib
import re
import sys


def main() -> None:
    args = sys.argv[1:]
    root = pathlib.Path(next(a for a in args if not a.startswith("--")))
    ggml = root / "servers" / "ggml"

    if "--msvc" in args:
        h = ggml / "src" / "zonos3.h"
        t = h.read_text(encoding="utf-8")
        if "struct probe_head" in t:
            print("msvc patch: already fixed upstream")
        else:
            patch_msvc(h, t)

    if "--backend-dl" in args:
        m = ggml / "src" / "server" / "main.cpp"
        t = m.read_text(encoding="utf-8")
        head = "int main(int argc, char ** argv) {\n"
        if t.count(head) != 1:
            sys.exit(f"backend-dl patch: expected one main(), found {t.count(head)}")
        shim = (
            '#include "ggml-backend.h"\n'
            "// Booth: with GGML_BACKEND_DL the CPU backend is a plugin library, so its exported helpers\n"
            "// aren't linkable. The encoders call this one; forward it through the backend registry.\n"
            'extern "C" void ggml_backend_cpu_set_n_threads(ggml_backend_t backend, int n_threads) {\n'
            "    ggml_backend_dev_t dev = backend ? ggml_backend_get_device(backend) : nullptr;\n"
            "    ggml_backend_reg_t reg = dev ? ggml_backend_dev_backend_reg(dev) : nullptr;\n"
            "    auto fn = reg ? (ggml_backend_set_n_threads_t) ggml_backend_reg_get_proc_address(reg, \"ggml_backend_set_n_threads\") : nullptr;\n"
            "    if (fn) fn(backend, n_threads);\n"
            "}\n"
        )
        load = "    ggml_backend_load_all(); // Booth: load the CPU backend variants next to the server\n"
        t = t.replace(head, shim + head + load)
        m.write_text(t, encoding="utf-8", newline="\n")
        print("patched", m)


def patch_msvc(h: pathlib.Path, t: str) -> None:
    t, n1 = re.subn(r"struct head \{ int layer = 0, head = 0; \};", "struct probe_head { int layer = 0, head = 0; };", t)
    t, n2 = re.subn(r"std::vector<head> heads;", "std::vector<probe_head> heads;", t)
    if n1 != 1 or n2 != 1:
        sys.exit(f"msvc patch: expected one match each, got {n1} and {n2}")
    h.write_text(t, encoding="utf-8", newline="\n")
    print("patched", h)


if __name__ == "__main__":
    main()
