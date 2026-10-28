# source-skim

`source-skim` compiles deterministic, provenance-preserving context manifests
under a token budget. It keeps required evidence, enforces source diversity,
selects remaining non-overlapping spans by score density, and records an exact
SHA-256 digest for both the input bundle and every selected byte range.

The verifier recomputes the selection from the original sources. Stale,
tampered, or manually edited manifests fail closed before text is rendered.

## Why

Long-context inference systems increasingly prune or route input before model
prefill. Saving tokens is useful only when operators can still answer: which
source bytes reached the model, were mandatory facts retained, and has the
source changed since selection? `source-skim` provides that small, auditable
boundary without depending on a model runtime.

## Install

```console
cargo install --git https://github.com/nicholas-ruest/source-skim
```

Rust 1.85 or newer is supported.

## Use

Compile a manifest from the included example:

```console
source-skim compile \
  --input examples/demo-bundle.json \
  --budget 22 \
  --min-sources 2 \
  --output manifest.json
```

Verify it against the exact source bundle:

```console
source-skim verify \
  --input examples/demo-bundle.json \
  --manifest manifest.json
```

Render verified text with explicit source and byte-range markers:

```console
source-skim render \
  --input examples/demo-bundle.json \
  --manifest manifest.json \
  --output context.txt
```

The input is a JSON object with `sources` and candidate byte ranges. Candidate
token counts and relevance scores come from the caller, so `source-skim` stays
tokenizer- and model-independent. Set `required: true` for evidence that must
survive selection. The command exits non-zero when required spans exceed the
budget, requested source diversity is impossible, a byte range splits UTF-8,
or any identifier, digest, policy field, or selection result is inconsistent.

## Selection contract

1. Validate source/candidate identities, UTF-8 byte boundaries, token counts,
   scores, and references.
2. Admit all non-overlapping required spans or fail.
3. Add the best eligible span from uncovered sources until `min_sources` is
   reached or fail.
4. Fill remaining budget by descending score-per-token, then deterministic
   tie-breakers; overlapping candidates are skipped.
5. Sort output by source and byte range, then hash the canonical bundle and
   each selected slice.

The algorithm is intentionally a deterministic greedy planner, not a learned
token gate and not an optimal knapsack solver. Token estimates and relevance
scores are trusted upstream inputs. Rendering includes provenance markers but
does not sanitize untrusted source text.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo +1.85.0 check --all-features
```

CI repeats the stable checks and the Rust 1.85 minimum-supported-version check.

## License

MIT
