# Local AI runtime and model licensing matrix

Status: research note compiled 2026-10-16 for #75 and #93. It records
verifiable licensing facts only; it is not legal advice, and it does not choose
a runtime or a baseline model.

Runtime code licensing and model-weight licensing are independent concerns and
are recorded separately below: a runtime's license never covers the models it
loads.

The candidates were selected from the requirements in #75 (embeddable in the
Rust/Tauri stack, fully offline, no separate server, workable on Apple Silicon,
Windows x64, and Linux). Replace or extend rows as the shortlist changes.

## Runtime candidates

| Runtime | License | Redistribution | Modification | Commercial use | Obligations | Primary source |
| --- | --- | --- | --- | --- | --- | --- |
| llama.cpp | MIT | Permitted | Permitted | Permitted | Include the copyright notice and license text | [LICENSE](https://github.com/ggml-org/llama.cpp/blob/master/LICENSE) |
| llamafile | Apache-2.0 (repository); bundled components such as llama.cpp remain under their own MIT license | Permitted | Permitted | Permitted | Include the license and NOTICE text; state modified files; keep the bundled components' notices | [LICENSE](https://github.com/Mozilla-Ocho/llamafile/blob/main/LICENSE) |
| ONNX Runtime | MIT | Permitted | Permitted | Permitted | Include the copyright notice and license text | [LICENSE](https://github.com/microsoft/onnxruntime/blob/main/LICENSE) |
| Candle | MIT OR Apache-2.0 (dual) | Permitted | Permitted | Permitted | Comply with one of the two alternatives | [LICENSE-MIT](https://github.com/huggingface/candle/blob/main/LICENSE-MIT) · [LICENSE-APACHE](https://github.com/huggingface/candle/blob/main/LICENSE-APACHE) |
| MLX | MIT | Permitted | Permitted | Permitted | Include the copyright notice and license text; Apple platforms only, so it cannot serve Windows or Linux | [LICENSE](https://github.com/ml-explore/mlx/blob/main/LICENSE) |

All runtime licenses above are permissive OSI-approved licenses. None of them
prevents bundling in an MIT-licensed application, and none adds a
source-disclosure obligation for OffPDF.

## Baseline-model candidates

Whether the redistribution obligations below apply depends on the exact flow
and on the model's terms. When a flow downloads weights directly from the
original provider at the user's request, OffPDF may not be redistributing them,
but that should be confirmed for each flow instead of assumed. The runtime
obligations above still apply regardless. The table covers both bundled and
download-on-first-run cases.

| Model (size class) | License | Redistribution | Modification | Commercial use | Notices and restrictions | Primary source |
| --- | --- | --- | --- | --- | --- | --- |
| Qwen3-4B (0.6B–4B dense) | Apache-2.0 | Permitted | Permitted | Permitted | Include the license text and NOTICE | [Model card](https://huggingface.co/Qwen/Qwen3-4B) |
| Phi-4-mini-instruct (3.8B) | MIT | Permitted | Permitted | Permitted | Include the copyright notice and license text | [Model card](https://huggingface.co/microsoft/Phi-4-mini-instruct) |
| SmolLM2-1.7B-Instruct (1.7B) | Apache-2.0 | Permitted | Permitted | Permitted | Include the license text and NOTICE | [Model card](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct) |
| Mistral-7B-Instruct-v0.3 (7B) | Apache-2.0 | Permitted | Permitted | Permitted | Include the license text and NOTICE | [Model card](https://huggingface.co/mistralai/Mistral-7B-Instruct-v0.3) |
| Llama 3.2 1B / 3B (text-only) | Llama 3.2 Community License (custom; not OSI-approved) | Permitted with pass-through | Permitted; derivative model names must begin with "Llama" | Permitted below 700M monthly active users | If redistributing: provide a copy of the license; prominently display "Built with Llama"; include the license's "Notice" file attribution; pass on the Acceptable Use Policy | [Model card](https://huggingface.co/meta-llama/Llama-3.2-1B) · [License text](https://github.com/meta-llama/llama-models/blob/main/models/llama3_2/LICENSE) · [Use policy](https://github.com/meta-llama/llama-models/blob/main/models/llama3_2/USE_POLICY.md) |
| Gemma 3 4B | Gemma Terms of Use (custom; not OSI-approved) | Permitted with pass-through | Permitted | Permitted | Pass the terms and the Prohibited Use Policy on to recipients | [Model card](https://huggingface.co/google/gemma-3-4b-it) · [Terms](https://ai.google.dev/gemma/terms) · [Prohibited use](https://ai.google.dev/gemma/prohibited_use_policy) |

## Imported files

The model store does not invent a license for a user-imported file. If the file
does not carry a license that OffPDF can read, Settings shows "Unknown — check
the model's license before using it" and never displays the store's internal
`imported` placeholder as a license name. The user remains responsible for
confirming the terms of any file they import.

## Items marked for review (not guessed)

- **Llama 3.2 geographic scope.** The release notes state that the multimodal
  variants are not offered in the EU
  ([announcement](https://ai.meta.com/blog/llama-3-2-connect-2024-vision-edge-mobile-devices/));
  the text models are not covered by that sentence, but the current license
  text should be read before any bundled distribution. This note deliberately
  does not conclude.
- **Gemma 3 obligations depend on the flow.** Whether recipients must be given
  the terms depends on whether OffPDF distributes the weights itself (bundling
  or mirroring) or has the app download them directly from Google; confirm the
  exact flow before bundling.
- **Qwen3 family scope.** The license is set per model; verify the model card
  of the exact revision added before treating this row as final for other
  sizes.
