//! Deterministic context selection with byte-level provenance.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use thiserror::Error;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub version: u32,
    pub sources: Vec<Source>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub source: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub tokens: u32,
    pub score: f64,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub token_budget: u32,
    pub min_sources: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub bundle_sha256: String,
    pub token_budget: u32,
    pub min_sources: usize,
    pub total_tokens: u32,
    pub sources_covered: usize,
    pub spans: Vec<ManifestSpan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSpan {
    pub candidate_id: String,
    pub source_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub tokens: u32,
    pub score: f64,
    pub required: bool,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub spans: usize,
    pub total_tokens: u32,
    pub sources_covered: usize,
}

#[derive(Debug, Error, PartialEq)]
pub enum SkimError {
    #[error("unsupported bundle version {0}; expected {FORMAT_VERSION}")]
    UnsupportedVersion(u32),
    #[error("token budget must be greater than zero")]
    ZeroBudget,
    #[error("duplicate source id: {0}")]
    DuplicateSource(String),
    #[error("duplicate candidate id: {0}")]
    DuplicateCandidate(String),
    #[error("candidate {candidate} references missing source {source_id}")]
    MissingSource {
        candidate: String,
        source_id: String,
    },
    #[error("candidate {candidate} has an invalid byte range for source {source_id}")]
    InvalidRange {
        candidate: String,
        source_id: String,
    },
    #[error("candidate {0} must have a positive token count")]
    ZeroTokens(String),
    #[error("candidate {0} must have a finite, non-negative score")]
    InvalidScore(String),
    #[error("required candidates overlap in source {source_id}: {left} and {right}")]
    RequiredOverlap {
        source_id: String,
        left: String,
        right: String,
    },
    #[error("required candidates need {required} tokens, exceeding budget {budget}")]
    RequiredOverBudget { required: u32, budget: u32 },
    #[error("cannot cover {requested} sources within budget; maximum achieved is {achieved}")]
    DiversityUnreachable { requested: usize, achieved: usize },
    #[error("manifest does not match the deterministic selection for this bundle and policy")]
    ManifestMismatch,
}

/// Compile a deterministic manifest. Required spans are admitted first, then
/// source diversity, then remaining candidates by score density.
pub fn compile(bundle: &Bundle, policy: Policy) -> Result<Manifest, SkimError> {
    let sources = validate(bundle, policy)?;
    let mut selected: Vec<&Candidate> = Vec::new();
    let mut used_tokens = 0_u32;

    let mut required: Vec<_> = bundle.candidates.iter().filter(|c| c.required).collect();
    required.sort_by(|a, b| a.id.cmp(&b.id));
    for candidate in required {
        if let Some(other) = selected.iter().find(|other| overlaps(candidate, other)) {
            return Err(SkimError::RequiredOverlap {
                source_id: candidate.source.clone(),
                left: other.id.clone(),
                right: candidate.id.clone(),
            });
        }
        used_tokens = used_tokens.saturating_add(candidate.tokens);
        selected.push(candidate);
    }
    if used_tokens > policy.token_budget {
        return Err(SkimError::RequiredOverBudget {
            required: used_tokens,
            budget: policy.token_budget,
        });
    }

    let mut covered: BTreeSet<&str> = selected.iter().map(|c| c.source.as_str()).collect();
    while covered.len() < policy.min_sources {
        let remaining = policy.token_budget - used_tokens;
        let next = ranked_candidates(bundle).into_iter().find(|candidate| {
            !covered.contains(candidate.source.as_str())
                && candidate.tokens <= remaining
                && !selected.iter().any(|other| overlaps(candidate, other))
        });
        let Some(candidate) = next else {
            return Err(SkimError::DiversityUnreachable {
                requested: policy.min_sources,
                achieved: covered.len(),
            });
        };
        used_tokens += candidate.tokens;
        covered.insert(candidate.source.as_str());
        selected.push(candidate);
    }

    for candidate in ranked_candidates(bundle) {
        if selected.iter().any(|chosen| chosen.id == candidate.id)
            || candidate.tokens > policy.token_budget - used_tokens
            || selected.iter().any(|other| overlaps(candidate, other))
        {
            continue;
        }
        used_tokens += candidate.tokens;
        covered.insert(candidate.source.as_str());
        selected.push(candidate);
    }

    selected.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(a.start_byte.cmp(&b.start_byte))
            .then(a.id.cmp(&b.id))
    });
    let spans = selected
        .into_iter()
        .map(|candidate| {
            let source = sources[&candidate.source];
            let content = &source.content[candidate.start_byte..candidate.end_byte];
            ManifestSpan {
                candidate_id: candidate.id.clone(),
                source_id: candidate.source.clone(),
                start_byte: candidate.start_byte,
                end_byte: candidate.end_byte,
                tokens: candidate.tokens,
                score: candidate.score,
                required: candidate.required,
                content_sha256: sha256(content.as_bytes()),
            }
        })
        .collect();

    Ok(Manifest {
        version: FORMAT_VERSION,
        bundle_sha256: bundle_digest(bundle),
        token_budget: policy.token_budget,
        min_sources: policy.min_sources,
        total_tokens: used_tokens,
        sources_covered: covered.len(),
        spans,
    })
}

/// Verify all digests, byte ranges, policy fields, and the deterministic
/// selection itself. A manually edited or stale manifest fails closed.
pub fn verify(bundle: &Bundle, manifest: &Manifest) -> Result<Verification, SkimError> {
    if manifest.version != FORMAT_VERSION || manifest.bundle_sha256 != bundle_digest(bundle) {
        return Err(SkimError::ManifestMismatch);
    }
    let expected = compile(
        bundle,
        Policy {
            token_budget: manifest.token_budget,
            min_sources: manifest.min_sources,
        },
    )?;
    if &expected != manifest {
        return Err(SkimError::ManifestMismatch);
    }
    Ok(Verification {
        spans: manifest.spans.len(),
        total_tokens: manifest.total_tokens,
        sources_covered: manifest.sources_covered,
    })
}

/// Render verified spans with unambiguous source/range markers.
pub fn render(bundle: &Bundle, manifest: &Manifest) -> Result<String, SkimError> {
    verify(bundle, manifest)?;
    let sources: BTreeMap<_, _> = bundle.sources.iter().map(|s| (&s.id, s)).collect();
    let mut output = String::new();
    for span in &manifest.spans {
        let source = sources[&span.source_id];
        let text = &source.content[span.start_byte..span.end_byte];
        output.push_str(&format!(
            "<<<source:{} bytes:{}-{} candidate:{}>>>\n{}\n<<<end>>>\n",
            span.source_id, span.start_byte, span.end_byte, span.candidate_id, text
        ));
    }
    Ok(output)
}

fn validate(bundle: &Bundle, policy: Policy) -> Result<BTreeMap<String, &Source>, SkimError> {
    if bundle.version != FORMAT_VERSION {
        return Err(SkimError::UnsupportedVersion(bundle.version));
    }
    if policy.token_budget == 0 {
        return Err(SkimError::ZeroBudget);
    }
    let mut sources = BTreeMap::new();
    for source in &bundle.sources {
        if sources.insert(source.id.clone(), source).is_some() {
            return Err(SkimError::DuplicateSource(source.id.clone()));
        }
    }
    let mut candidate_ids = HashSet::new();
    for candidate in &bundle.candidates {
        if !candidate_ids.insert(&candidate.id) {
            return Err(SkimError::DuplicateCandidate(candidate.id.clone()));
        }
        let Some(source) = sources.get(&candidate.source) else {
            return Err(SkimError::MissingSource {
                candidate: candidate.id.clone(),
                source_id: candidate.source.clone(),
            });
        };
        if candidate.start_byte >= candidate.end_byte
            || candidate.end_byte > source.content.len()
            || !source.content.is_char_boundary(candidate.start_byte)
            || !source.content.is_char_boundary(candidate.end_byte)
        {
            return Err(SkimError::InvalidRange {
                candidate: candidate.id.clone(),
                source_id: candidate.source.clone(),
            });
        }
        if candidate.tokens == 0 {
            return Err(SkimError::ZeroTokens(candidate.id.clone()));
        }
        if !candidate.score.is_finite() || candidate.score < 0.0 {
            return Err(SkimError::InvalidScore(candidate.id.clone()));
        }
    }
    Ok(sources)
}

fn ranked_candidates(bundle: &Bundle) -> Vec<&Candidate> {
    let mut candidates: Vec<_> = bundle.candidates.iter().collect();
    candidates.sort_by(|a, b| rank_cmp(a, b));
    candidates
}

fn rank_cmp(a: &Candidate, b: &Candidate) -> Ordering {
    let a_density = a.score / f64::from(a.tokens);
    let b_density = b.score / f64::from(b.tokens);
    b_density
        .total_cmp(&a_density)
        .then_with(|| b.score.total_cmp(&a.score))
        .then_with(|| a.tokens.cmp(&b.tokens))
        .then_with(|| a.id.cmp(&b.id))
}

fn overlaps(a: &Candidate, b: &Candidate) -> bool {
    a.source == b.source && a.start_byte < b.end_byte && b.start_byte < a.end_byte
}

fn bundle_digest(bundle: &Bundle) -> String {
    let bytes = serde_json::to_vec(bundle).expect("serializing Bundle cannot fail");
    sha256(&bytes)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> Bundle {
        Bundle {
            version: 1,
            sources: vec![
                Source {
                    id: "alpha".into(),
                    content: "alpha evidence one\nalpha evidence two".into(),
                },
                Source {
                    id: "beta".into(),
                    content: "beta required\nbeta optional".into(),
                },
            ],
            candidates: vec![
                Candidate {
                    id: "a1".into(),
                    source: "alpha".into(),
                    start_byte: 0,
                    end_byte: 18,
                    tokens: 4,
                    score: 0.8,
                    required: false,
                },
                Candidate {
                    id: "a2".into(),
                    source: "alpha".into(),
                    start_byte: 19,
                    end_byte: 37,
                    tokens: 4,
                    score: 0.6,
                    required: false,
                },
                Candidate {
                    id: "b1".into(),
                    source: "beta".into(),
                    start_byte: 0,
                    end_byte: 13,
                    tokens: 3,
                    score: 0.2,
                    required: true,
                },
                Candidate {
                    id: "b2".into(),
                    source: "beta".into(),
                    start_byte: 14,
                    end_byte: 27,
                    tokens: 3,
                    score: 0.9,
                    required: false,
                },
            ],
        }
    }

    #[test]
    fn selects_required_diverse_and_high_density_spans() {
        let manifest = compile(
            &bundle(),
            Policy {
                token_budget: 10,
                min_sources: 2,
            },
        )
        .unwrap();
        let ids: Vec<_> = manifest
            .spans
            .iter()
            .map(|span| span.candidate_id.as_str())
            .collect();
        assert_eq!(ids, ["a1", "b1", "b2"]);
        assert_eq!(manifest.total_tokens, 10);
        assert_eq!(manifest.sources_covered, 2);
    }

    #[test]
    fn output_is_independent_of_candidate_input_order() {
        let first = bundle();
        let mut second = bundle();
        second.candidates.reverse();
        let policy = Policy {
            token_budget: 10,
            min_sources: 2,
        };
        let a = compile(&first, policy).unwrap();
        let b = compile(&second, policy).unwrap();
        assert_eq!(a.spans, b.spans);
        assert_ne!(a.bundle_sha256, b.bundle_sha256);
    }

    #[test]
    fn rejects_tampered_manifest() {
        let bundle = bundle();
        let mut manifest = compile(
            &bundle,
            Policy {
                token_budget: 10,
                min_sources: 2,
            },
        )
        .unwrap();
        manifest.spans[0].content_sha256.replace_range(0..1, "0");
        assert_eq!(verify(&bundle, &manifest), Err(SkimError::ManifestMismatch));
    }

    #[test]
    fn required_spans_fail_when_they_exceed_budget() {
        let mut bundle = bundle();
        bundle.candidates[0].required = true;
        let error = compile(
            &bundle,
            Policy {
                token_budget: 5,
                min_sources: 1,
            },
        )
        .unwrap_err();
        assert_eq!(
            error,
            SkimError::RequiredOverBudget {
                required: 7,
                budget: 5
            }
        );
    }

    #[test]
    fn validates_utf8_boundaries() {
        let bundle = Bundle {
            version: 1,
            sources: vec![Source {
                id: "unicode".into(),
                content: "A🦀B".into(),
            }],
            candidates: vec![Candidate {
                id: "cut".into(),
                source: "unicode".into(),
                start_byte: 1,
                end_byte: 2,
                tokens: 1,
                score: 1.0,
                required: false,
            }],
        };
        assert!(matches!(
            compile(
                &bundle,
                Policy {
                    token_budget: 1,
                    min_sources: 0
                }
            ),
            Err(SkimError::InvalidRange { .. })
        ));
    }

    #[test]
    fn renders_only_after_verification() {
        let bundle = bundle();
        let manifest = compile(
            &bundle,
            Policy {
                token_budget: 7,
                min_sources: 2,
            },
        )
        .unwrap();
        let output = render(&bundle, &manifest).unwrap();
        assert!(output.contains("source:alpha"));
        assert!(output.contains("source:beta"));
        assert!(output.contains("alpha evidence one"));
    }
}
