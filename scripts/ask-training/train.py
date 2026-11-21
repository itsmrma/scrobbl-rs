#!/usr/bin/env python3
"""Fine-tune Qwen3.5 2B to write scrobbl.rs Ask SQL, then export a GGUF.

Input: the `train.jsonl` / `validation.jsonl` files written by
`cargo run --no-default-features --bin ask-train-data -- --out <dir>`.
Each line carries `prompt` (the exact runtime ChatML prompt, ending with the
empty Qwen3.5 thinking block) and `completion` (the JSON answer + `<|im_end|>`).
Loss is computed on the completion only, so the model learns to answer, not to
repeat the schema.

Recipe (see docs/ask-training.md): bf16 LoRA, r=32, all attention and MLP
projections, 3 epochs, cosine schedule. Unsloth advises against 4-bit QLoRA for
Qwen3.5, so the base weights stay in 16 bit (about 5 GB VRAM for 2B at short
context; budget ~12-16 GB at the 6k-token prompts used here).

Usage:
  python train.py --data ./ask-sft --out ./ask-qwen35-2b-sql
  python train.py --data ./ask-sft --out ./ask-qwen35-2b-sql --epochs 2 --lr 1e-4

The script never reads an archive: the dataset is synthetic by construction.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--data", required=True, type=Path, help="folder with train.jsonl and validation.jsonl")
    parser.add_argument("--out", required=True, type=Path, help="output folder for adapters, merged model and GGUF")
    parser.add_argument("--base", default="unsloth/Qwen3.5-2B", help="Hugging Face id of the base model")
    parser.add_argument("--max-seq-length", type=int, default=6144)
    parser.add_argument("--epochs", type=float, default=3.0)
    parser.add_argument("--lr", type=float, default=2e-4)
    parser.add_argument("--rank", type=int, default=32)
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--grad-accum", type=int, default=16)
    parser.add_argument("--seed", type=int, default=3407)
    parser.add_argument("--quant", default="q4_k_m", choices=["q4_k_m", "q8_0", "f16"])
    parser.add_argument("--skip-gguf", action="store_true", help="only save the LoRA adapter")
    return parser.parse_args()


def load_split(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            record = json.loads(line)
            # The prompt already ends with the assistant header and the empty
            # thinking block; never re-apply a chat template on top of it.
            rows.append({"prompt": record["prompt"], "completion": record["completion"]})
    return rows


def main() -> None:
    args = parse_args()
    # Imported here so `--help` works without the GPU stack installed.
    from datasets import Dataset
    from trl import SFTConfig, SFTTrainer
    from unsloth import FastLanguageModel

    train_rows = load_split(args.data / "train.jsonl")
    eval_rows = load_split(args.data / "validation.jsonl")
    print(f"train={len(train_rows)} validation={len(eval_rows)}")

    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=args.base,
        max_seq_length=args.max_seq_length,
        load_in_4bit=False,
        load_in_16bit=True,
        full_finetuning=False,
    )
    model = FastLanguageModel.get_peft_model(
        model,
        r=args.rank,
        lora_alpha=args.rank,
        lora_dropout=0,
        bias="none",
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
        use_gradient_checkpointing="unsloth",
        random_state=args.seed,
        max_seq_length=args.max_seq_length,
    )

    # Qwen3.5 is multimodal: Unsloth returns a processor whose first positional
    # argument is an image. Text-only work goes through its inner tokenizer.
    text_tokenizer = getattr(tokenizer, "tokenizer", tokenizer)

    longest = max(len(text_tokenizer(text=r["prompt"] + r["completion"])["input_ids"]) for r in train_rows[:200])
    if longest > args.max_seq_length:
        raise SystemExit(f"examples reach {longest} tokens; raise --max-seq-length")

    trainer = SFTTrainer(
        model=model,
        processing_class=text_tokenizer,
        train_dataset=Dataset.from_list(train_rows),
        eval_dataset=Dataset.from_list(eval_rows),
        args=SFTConfig(
            output_dir=str(args.out / "checkpoints"),
            max_length=args.max_seq_length,
            completion_only_loss=True,
            per_device_train_batch_size=args.batch_size,
            gradient_accumulation_steps=args.grad_accum,
            num_train_epochs=args.epochs,
            learning_rate=args.lr,
            lr_scheduler_type="cosine",
            warmup_steps=0.03,  # transformers>=5: a float below 1 is a ratio of total steps
            weight_decay=0.01,
            optim="adamw_8bit",
            logging_steps=10,
            eval_strategy="steps",
            eval_steps=100,
            save_strategy="steps",
            save_steps=100,
            save_total_limit=2,
            load_best_model_at_end=True,
            metric_for_best_model="eval_loss",
            bf16=True,
            seed=args.seed,
            dataset_num_proc=1,
            report_to="none",
        ),
    )
    trainer.train()

    adapter = args.out / "lora"
    model.save_pretrained(str(adapter))
    tokenizer.save_pretrained(str(adapter))
    print(f"LoRA adapter saved to {adapter}")
    if args.skip_gguf:
        return

    gguf_dir = args.out / "gguf"
    model.save_pretrained_gguf(str(gguf_dir), tokenizer, quantization_method=args.quant)
    produced = sorted(p for p in gguf_dir.glob("*.gguf") if args.quant.upper() in p.name.upper())
    print("GGUF written:", ", ".join(str(p) for p in produced) or os.listdir(gguf_dir))
    print("Next: evaluate it with `cargo run --release --bin ask-eval -- --model <file.gguf>`.")


if __name__ == "__main__":
    main()
