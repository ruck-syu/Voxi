#!/usr/bin/env python3
"""Export the frozen HF T5 checkpoint to Candle-compatible Q8_0 GGUF."""
import argparse
import json
import shutil
from pathlib import Path

import numpy as np
from safetensors import safe_open
import gguf


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--source", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    src, out = args.source, args.output
    out.mkdir(parents=True, exist_ok=True)
    config = json.loads((src / "config.json").read_text())
    if config["model_type"] != "t5" or not config["tie_word_embeddings"]:
        raise SystemExit("expected tied-embedding T5 checkpoint")

    writer = gguf.GGUFWriter(str(out / "model-q8_0.gguf"), "t5")
    writer.add_name("Voxi T5 Efficient Tiny v3")
    writer.add_description("Local Q8_0 export; tied embeddings; Candle quantized_t5")
    writer.add_tokenizer_model("t5")
    writer.add_vocab_size(config["vocab_size"])
    writer.add_block_count(config["num_layers"])
    writer.add_decoder_block_count(config["num_decoder_layers"])
    writer.add_embedding_length(config["d_model"])
    writer.add_feed_forward_length(config["d_ff"])
    writer.add_head_count(config["num_heads"])
    writer.add_key_length(config["d_kv"])
    writer.add_relative_attn_buckets_count(config["relative_attention_num_buckets"])
    writer.add_layer_norm_rms_eps(config["layer_norm_epsilon"])
    writer.add_eos_token_id(config["eos_token_id"])
    writer.add_pad_token_id(config["pad_token_id"])
    writer.add_decoder_start_token_id(config["decoder_start_token_id"])

    with safe_open(src / "model.safetensors", framework="np") as sf:
        names = set(sf.keys())
        # Candle's quantized T5 loader requests the original HF matrix shape
        # ([out,in]) from its GGUF variable builder.
        for name in sorted(names):
            if name == "lm_head.weight":
                continue  # tied to shared.weight by config
            a = np.asarray(sf.get_tensor(name), dtype=np.float32)
            if a.ndim == 2 and a.shape[-1] % 32 == 0:
                qtype = gguf.GGMLQuantizationType.Q8_0
                # Q8_0 quantizer operates per row; all these T5 inner dims are
                # multiples of 32, including the shared vocabulary embedding.
                quant = gguf.quants.quantize(a, qtype)
                writer.add_tensor(name, quant, raw_dtype=qtype)
            else:
                writer.add_tensor(name, a)
    writer.write_header_to_file()
    writer.write_kv_data_to_file()
    writer.write_tensors_to_file(progress=True)
    writer.close()
    shutil.copy2(src / "tokenizer.json", out / "tokenizer.json")
    shutil.copy2(src / "config.json", out / "config.json")


if __name__ == "__main__":
    main()
