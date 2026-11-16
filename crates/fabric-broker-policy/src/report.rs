//! Digests, the proof id, and the contract's proof report and ruling for a check
//! (`contracts/desktop/v1/README.md` sections 4 and 9; plan section 8).

use fabric_desktop_contracts::canonical;
use fabric_desktop_contracts::digest::hex;
use fabric_desktop_contracts::owner::{
    ProofOutcome, ProofPurpose, ProofReport, RuleVerdict, Ruling, Verdict as ContractVerdict,
};
use fabric_desktop_contracts::scalar::{
    Count, Digest, Explanation, Id, Label, List, NameToken, ScalarError, Timestamp,
};
use sha2::{Digest as _, Sha256};

use crate::check::{RuleOutcome, Verdict};
use crate::gate::Check;
use crate::model::{Context, Outcome};

/// The domain tag of the proof id.
pub const PROOF_DOMAIN: &str = "fabric-broker-proof-v1";

/// SHA-256 of `bytes` in lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The context digest: `"sha256:" + hex(SHA-256("fabric-desktop/1\ncontext\n" +
/// canonical(context)))`, the contract's digest formula with the domain `context`
/// (README section 4).
#[must_use]
pub fn context_digest(context: &Context) -> String {
    let value = serde_json::to_value(context).expect("a context serializes to JSON");
    let mut hasher = Sha256::new();
    hasher.update(format!("{}\ncontext\n", fabric_desktop_contracts::PROTOCOL).as_bytes());
    hasher.update(canonical(&value).as_bytes());
    format!("sha256:{}", hex(&hasher.finalize()))
}

/// The proof id: SHA-256 over the domain tag, the theory hash, the Isabelle version and the
/// check theory text, each followed by a line feed but the last; 64 hex characters.
#[must_use]
pub fn proof_id(theory_sha256: &str, isabelle: &str, check_theory: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [PROOF_DOMAIN, theory_sha256, isabelle] {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(check_theory.as_bytes());
    hex(&hasher.finalize())
}

const fn contract_verdict(outcome: RuleOutcome) -> ContractVerdict {
    match outcome {
        RuleOutcome::Holds => ContractVerdict::Holds,
        RuleOutcome::Fails => ContractVerdict::Fails,
        RuleOutcome::Undecided => ContractVerdict::Undecided,
    }
}

/// The contract's `proof_report` for a check (`owner#/$defs/proof_report`). It holds digests
/// and verdicts, never calendar content.
pub fn proof_report(
    check: &Check,
    request_id: &Id,
    purpose: ProofPurpose,
    started_at: &Timestamp,
) -> Result<ProofReport, ScalarError> {
    let outcome = match check.verdict {
        Verdict::Decided {
            outcome: Outcome::Permit,
        } => ProofOutcome::Permit,
        Verdict::Decided {
            outcome: Outcome::NeedApproval,
        } => ProofOutcome::NeedApproval,
        Verdict::Decided {
            outcome: Outcome::Block,
        } => ProofOutcome::Block,
        Verdict::Undecided { .. } => ProofOutcome::Undecided,
    };
    let rule_verdicts = check
        .rules
        .iter()
        .map(|result| {
            Ok(RuleVerdict {
                rule: NameToken::new(result.rule.name())?,
                verdict: contract_verdict(result.outcome),
            })
        })
        .collect::<Result<Vec<_>, ScalarError>>()?;
    let elapsed_ms = (check.seconds * 1000.0).round();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a check's duration is a small non-negative number of milliseconds"
    )]
    let elapsed_ms = if elapsed_ms.is_finite() && elapsed_ms > 0.0 {
        elapsed_ms as u64
    } else {
        0
    };
    Ok(ProofReport {
        proof_id: Id::new(check.proof_id.as_str())?,
        request_id: request_id.clone(),
        purpose,
        outcome,
        failing_rule: check
            .failing_rule
            .map(|rule| NameToken::new(rule.name()))
            .transpose()?,
        rule_verdicts: List::new(rule_verdicts)?,
        isabelle_version: Label::new(check.isabelle.as_str())?,
        theory_hash: Digest::new(format!("sha256:{}", check.theory_sha256))?,
        context_digest: Digest::new(check.context_digest.as_str())?,
        action_digest: Digest::new(format!("sha256:{}", check.action_digest))?,
        started_at: started_at.clone(),
        elapsed_ms: Count::new(elapsed_ms)?,
    })
}

/// The contract's `ruling` for a check that proved Block or Need_approval: the first failing
/// rule, its sentence and the proof id (DEF-7). A permit and an undecided check have none; an
/// undecided request carries no rule (README section 9).
pub fn ruling(check: &Check) -> Result<Option<Ruling>, ScalarError> {
    match (&check.verdict, check.failing_rule) {
        (
            Verdict::Decided {
                outcome: Outcome::Block | Outcome::NeedApproval,
            },
            Some(rule),
        ) => Ok(Some(Ruling {
            rule: NameToken::new(rule.name())?,
            explanation: Explanation::new(rule.explanation())?,
            proof_id: Id::new(check.proof_id.as_str())?,
        })),
        _ => Ok(None),
    }
}
