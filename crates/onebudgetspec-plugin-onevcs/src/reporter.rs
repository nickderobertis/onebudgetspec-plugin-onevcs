//! `PublishRequest` to `DeliveryResult`: one standing comment, read once and upserted.
use onebudgetspec_core::delivery::{
    DeliveryResult, PublishRequest, Reporter, merge_standing_report, validate_request,
};
use onevcs::{
    ChangeRef, Providers, StandingCommentOutcome, StandingCommentRequest, StandingCommentStatus,
    host_query::CommitSha, read_standing_comment, upsert_standing_comment,
};

/// GitHub's comment limit is 65536 characters including onevcs's key marker, which is
/// under 128 bytes. Core measures bytes, and a character is at least one byte.
const MAX_BODY_BYTES: usize = 65_536 - 128;

/// Enough of a failure to act on. A failed `gh` call quotes its whole command line,
/// which for a write holds the comment body, so the middle of a long reason goes.
const DIAGNOSTIC_HEAD_CHARS: usize = 200;
const DIAGNOSTIC_TAIL_CHARS: usize = 600;

/// The reporter the default invocation runs: real git and the real host.
pub struct OnevcsReporter;

impl Reporter for OnevcsReporter {
    fn publish(&self, request: &PublishRequest) -> DeliveryResult {
        deliver(&Providers::real(), request)
    }
}

/// Merge the request into its standing comment and upsert it against its measured head.
///
/// Never panics or replaces a budget verdict: a stale head, a refused permission or a
/// failed post answers `delivered: false` with the reason.
#[must_use]
pub fn deliver(providers: &Providers<'_>, request: &PublishRequest) -> DeliveryResult {
    let change = ChangeRef::Url(request.destination.clone());
    let attempted = validate_request(request)
        .and_then(|()| {
            request
                .head_sha
                .parse::<CommitSha>()
                .map(drop)
                .map_err(|reason| format!("head_sha: {reason}"))
        })
        .and_then(|()| {
            read_standing_comment(providers, &change, &request.key)
                .map_err(|error| format!("could not read the standing comment: {error}"))
        })
        .and_then(|previous| {
            let body = merge_standing_report(
                previous.as_ref().map(|comment| comment.body.as_str()),
                request,
                MAX_BODY_BYTES,
            );
            upsert_standing_comment(
                providers,
                &StandingCommentRequest {
                    change,
                    key: request.key.clone(),
                    body,
                    expected_head: Some(request.head_sha.clone()),
                },
            )
            .map_err(|error| format!("could not post the standing comment: {error}"))
        });
    match attempted {
        Ok(outcome) => answer(outcome),
        Err(reason) => undelivered(&reason),
    }
}

/// The answer for a request that never parsed: the protocol still gets its one JSON
/// value, and the caller's budget verdict is still its own.
#[must_use]
pub fn undelivered(reason: &str) -> DeliveryResult {
    DeliveryResult {
        delivered: false,
        url: None,
        diagnostic: Some(bounded(reason)),
    }
}

fn answer(outcome: StandingCommentOutcome) -> DeliveryResult {
    if outcome.status == StandingCommentStatus::Stale {
        return undelivered(
            outcome
                .diagnostic
                .as_deref()
                .unwrap_or("the change request moved past the measured head; no comment written"),
        );
    }
    DeliveryResult {
        delivered: true,
        url: outcome.comment.map(|comment| comment.url),
        diagnostic: None,
    }
}

fn bounded(reason: &str) -> String {
    let characters: Vec<char> = reason.chars().collect();
    if characters.len() <= DIAGNOSTIC_HEAD_CHARS + DIAGNOSTIC_TAIL_CHARS {
        return reason.to_owned();
    }
    let head: String = characters[..DIAGNOSTIC_HEAD_CHARS].iter().collect();
    let tail: String = characters[characters.len() - DIAGNOSTIC_TAIL_CHARS..]
        .iter()
        .collect();
    format!("{head} … {tail}")
}
