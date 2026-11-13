# Model files Booth builds itself

Booth downloads ZONOS3-Mini's model files from Hugging Face at the commit pinned in
`crates/booth-core/models.lock.json`. A file here replaces the Hugging Face one when Zyphra has
changed it on GitHub but not yet uploaded the new build. Each one is made with Zyphra's own tools,
and differs from the Hugging Face file only as described below. `scripts/models-lock.mjs` lists them
in `OVERRIDES`; the mirror workflow uploads them to the mirror release, Booth's only source for them.
When Hugging Face gets the new file, the daily watch reports it: delete the override and re-pin.

## steer.gguf

The steering bundle, built from [Zyphra/ZONOS3-Mini@bcb6661](https://github.com/Zyphra/ZONOS3-Mini/tree/bcb66617512e4d09082438ade8be764af3213327)
(Apache-2.0, by Zyphra):

    PYTHONUTF8=1 PYTHONPATH="model/src;tools" python tools/convert_gguf.py steer \
      assets/steering/studio_bundle_mect60k_x10.json -o steer.gguf

Same 65 direction tensors and metadata as Hugging Face's `gguf/steer.gguf`, plus the
`zonos3_steer.steer_release` key from that commit ("steer then hold": request-level controls at full
dose for 4 s, then a 1 s ramp to 0.35x), which stops long lines from drifting.
