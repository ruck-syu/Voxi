# DRAFT: Tiny model versus production parser

## Method

Frozen corpus SHA-256: `851900248b9706c151eb94f9547a65d3f9541c43e8cc3ecbf0d0684ee1cd0fc8` (300 unchanged commands and heuristic expectations). Source replay used pinned production `v0.1.14` / `44c74fa65fd0b4a8f9a9ad2c79d435e7c30e3ffb` and the captured dev working tree, with isolated fixtures and scheduler time 2026-11-16 12:00 IST. No UI or execution module was invoked.

T5 Efficient Tiny v3 ran with the preserved local Python runtime, PyTorch CPU FP32, two threads, seed 0, local-only weights, greedy decoding, 64 token input/output limits, batch size 1. All 300 original commands were inferred. Total time: 9.402s; truncated inputs: 0; model inventory unchanged. Predictions include timings and token lengths.

Raw predictions were replayed as command rewrites against dev parser/scheduler/planner for diagnosis only. Guarded fallback eligibility came exclusively from dev `parse_classified == GrammarMiss`; accepted rules win and typed hard, schedule, and application refusals are terminal. The frozen copy guard, frozen conservative semantic guard, exact schedule preservation, and dev parser acceptance were required. Candidate planner results remain recorded when rejected.

## Results

Production stages: 227 accepted, 70 parser refusals, 3 schedule refusals. Dev rules: 224 accepted, 73 parser refusals, of which 49 are typed grammar misses and 24 hard refusals; 3 schedule refusals. Raw diagnostic: 243 accepted, 56 parser refusals, 1 schedule refusals.

Guarded decisions retained 224 rule outputs; 24 hard refusals and 3 schedule refusals remained terminal. Of 49 grammar misses, 9 failed the copy guard, 10 failed the semantic guard (overlap possible), 24 generated commands still failed parsing, and 12 passed diagnostic guards and parsed. 1 eligible candidate(s) had a different extracted schedule; rejection decisions can overlap. See `summary.json` and per-case result JSONL for all decisions and planner evidence.

Fresh rule outputs matched existing outputs after temporary fixture path normalization and removal of the new typed-classification/refusal-schedule metadata, except five rows in each version: {"dev": {"deviation_ids": ["T050", "T051", "T098", "S043", "S063"], "matches": 295}, "production": {"deviation_ids": ["T050", "T051", "T098", "S043", "S063"], "matches": 295}}. T050/T051/T098/S063 differ in wall-clock-derived resize backup directory names; S043 differs only in ordering of equally sized rows. These deviations are retained, not normalized away. The raw/guarded initial replay mistakenly used the last-built production executable; the parent corrected this by pinning separate dev/production binaries, retained the superseded outputs, and repeated source replay without changing any model predictions. Parser refusals now retain extracted schedule metadata. Final outputs use the captured dev binary for model rewrites.

## Limits

Expectations are heuristic, not gold labels; no accuracy percentage is reported. Agreement is not accuracy, and parser acceptance, no-match, or planner blocking does not establish safety. These are fixture source replays, not installed-app runtime validation. Production predates `parse_classified`; eligibility was determined only by the dev typed API. The earlier separate rules-versus-rules audit completed actual UI testing after access was enabled; model inference remains offline only. Semantic review remains separate. No post-inference tuning occurred. Raw diagnostic output does not authorize fallback.
