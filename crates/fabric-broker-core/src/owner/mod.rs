//! The owner channel (design 6): the 13 methods, command deduplication, the views and the
//! subscription ring. Crate-private; `crate::broker` calls it.
//!
//! Every answer is built from committed state: a command's notices are rung and written first,
//! then its response. A failed read or write answers the frame in hand `store_unavailable` and
//! hands the fault to the broker, which halts (5.2).

use std::collections::VecDeque;

use fabric_desktop_contracts::error::OwnerFault;
use fabric_desktop_contracts::owner::{
    Change, OwnerCall, OwnerMethod, OwnerNotice, OwnerRequest, OwnerResponse, OwnerResult,
    SubscribeResult,
};
use fabric_desktop_contracts::{Entry, decode};
use fabric_desktop_contracts::{Frame, Limits, MAX_FRAME_BYTES};
use fabric_storage::StoreFault;

use crate::service::core::Core;

pub(crate) mod commands;
pub(crate) mod dedup;
pub(crate) mod frames;
pub(crate) mod groups;
pub(crate) mod preferences;
pub(crate) mod profiles;
pub(crate) mod subscribe;
#[cfg(all(test, debug_assertions))]
mod tests;
pub(crate) mod views;

/// The answer to a line that decodes as another kind of frame.
const NOT_A_REQUEST_MESSAGE: &str = "The broker decoded an owner request as another kind of frame.";

/// The owner channel's state: the subscription and the ring of recent notices.
pub(crate) struct OwnerChannel {
    subscribed: bool,
    ever_subscribed: bool,
    /// At most 256 notices.
    ring: VecDeque<OwnerNotice>,
    base: u64,
    build: String,
}

/// True when the frame's line, without its line feed, is within the frame bound.
fn frame_fits(frame: &Frame) -> bool {
    frame.to_line().len() <= MAX_FRAME_BYTES + 1
}

/// The owner request a line holds, or the answer that refuses it.
fn request_of(line: &[u8]) -> Result<OwnerRequest, Box<Frame>> {
    match decode(Entry::OwnerRequest, line) {
        Ok(Frame::OwnerRequest(request)) => Ok(request),
        Ok(_) => Err(Box::new(frames::error_frame(
            None,
            None,
            OwnerFault::Internal,
            NOT_A_REQUEST_MESSAGE,
        ))),
        Err(refusal) => Err(Box::new(frames::refusal_answer(line, &refusal))),
    }
}

impl OwnerChannel {
    /// A channel with no subscription.
    pub(crate) fn new(build: String) -> OwnerChannel {
        OwnerChannel {
            subscribed: false,
            ever_subscribed: false,
            ring: VecDeque::with_capacity(subscribe::RING_CAPACITY),
            base: 0,
            build,
        }
    }

    /// Called after every `Core::open`: empties the ring, sets its base to `notice_sequence`, ends the subscription,
    /// then rings the recovery's notices.
    pub(crate) fn begin_run(&mut self, notice_sequence: u64, notices: Vec<OwnerNotice>) {
        self.reset_ring(notice_sequence);
        self.ring_notices(&notices);
    }

    /// One owner line against a ready core: notices committed by it first, then the response; the fault on a halt.
    ///
    /// `_now_ms` is the step's `now`, which the core already holds for every write.
    pub(crate) fn answer(
        &mut self,
        core: &mut Core,
        line: &[u8],
        _now_ms: i64,
    ) -> (Vec<Frame>, Option<StoreFault>) {
        let request = match request_of(line) {
            Ok(request) => request,
            Err(refusal) => return (vec![*refusal], None),
        };
        let mut out = Vec::new();
        match self.serve(core, &request, &mut out) {
            Ok(()) => (out, None),
            Err(fault) => {
                out.push(frames::halted(&request.id, request.call.method()));
                (out, Some(fault))
            }
        }
    }

    /// Answers one request into `out`.
    fn serve(
        &mut self,
        core: &mut Core,
        request: &OwnerRequest,
        out: &mut Vec<Frame>,
    ) -> Result<(), StoreFault> {
        let frame_id = request.id.clone();
        match &request.call {
            OwnerCall::Hello(params) => out.push(frames::hello(
                frame_id,
                params,
                &self.build,
                core.settings().limits,
            )),
            OwnerCall::GetSnapshot(_) => out.push(views::snapshot(core, frame_id)?),
            OwnerCall::GetRequestDetails(params) => {
                out.push(views::details(core, frame_id, params)?);
            }
            OwnerCall::GetProof(params) => out.push(views::proof(core, frame_id, params)?),
            OwnerCall::Subscribe(params) => {
                let notice_sequence = core.read(|r| r.meta())?.notice_sequence;
                match self.subscribe_at(params.cursor.get(), notice_sequence) {
                    Ok(notices) => {
                        out.push(frames::result_frame(
                            frame_id,
                            OwnerResult::Subscribe(SubscribeResult {
                                cursor: params.cursor,
                            }),
                        ));
                        out.extend(notices.into_iter().map(Frame::OwnerNotice));
                    }
                    Err(message) => out.push(frames::refuse(
                        &frame_id,
                        OwnerMethod::Subscribe,
                        OwnerFault::SnapshotRequired,
                        message,
                    )),
                }
            }
            // Not stored: the method exists but this build cannot run it (5.3, CCR-1).
            OwnerCall::ConnectGoogle(_) => out.push(frames::refuse(
                &frame_id,
                OwnerMethod::ConnectGoogle,
                OwnerFault::UnknownMethod,
                frames::CONNECT_GOOGLE_MESSAGE,
            )),
            // not implemented in this build (contract schema version 3)
            OwnerCall::SelectCalendars(_)
            | OwnerCall::ResolveOutcome(_)
            | OwnerCall::DecideEnrollment(_)
            | OwnerCall::RemoveAgent(_)
            | OwnerCall::ListEndedRequests(_) => out.push(frames::refuse(
                &frame_id,
                request.call.method(),
                OwnerFault::UnknownMethod,
                frames::NOT_IN_THIS_BUILD_MESSAGE,
            )),
            OwnerCall::SaveProfile(_)
            | OwnerCall::SavePreferences(_)
            | OwnerCall::SaveGroup(_)
            | OwnerCall::DecideRequest(_)
            | OwnerCall::PauseAgent(_)
            | OwnerCall::RevokeGrant(_)
            | OwnerCall::DisconnectGoogle(_) => {
                let (notices, answer) = dedup::command(core, request)?;
                out.extend(self.publish(notices));
                out.push(answer);
            }
        }
        Ok(())
    }

    /// The unavailable mode: `hello` normally, every other method `store_unavailable` with `reason.message()`.
    pub(crate) fn answer_unavailable(
        &mut self,
        reason: &fabric_storage::Unavailable,
        line: &[u8],
    ) -> Vec<Frame> {
        let request = match request_of(line) {
            Ok(request) => request,
            Err(refusal) => return vec![*refusal],
        };
        let frame = match &request.call {
            // No core holds the settings here; the limits are the only ones this build has.
            OwnerCall::Hello(params) => {
                frames::hello(request.id, params, &self.build, Limits::CD_G0)
            }
            call => frames::store_unavailable(
                Some(request.id.clone()),
                frames::method_text(call.method()),
                reason.store_reason(),
                &reason.message(),
            ),
        };
        vec![frame]
    }

    /// The refusal of an oversized line.
    pub(crate) fn oversized(&self) -> Frame {
        frames::oversized()
    }

    /// Ring the notices; return them as frames while subscribed (or one snapshot_required change past 256).
    pub(crate) fn publish(&mut self, notices: Vec<OwnerNotice>) -> Vec<Frame> {
        self.ring_notices(&notices);
        self.notice_frames(notices)
    }

    /// Keeps every owner frame within the frame bound (README section 1, plan 9.1), so the
    /// relay never sees an oversized broker line: a response longer than `MAX_FRAME_BYTES`
    /// becomes `internal` with its id and method; a notice longer than it becomes one
    /// `snapshot_required` change with its sequence, which ends the subscription, and the
    /// notices after it are not written.
    pub(crate) fn fit_frames(&mut self, frames: Vec<Frame>) -> Vec<Frame> {
        let mut out = Vec::with_capacity(frames.len());
        let mut ended = false;
        for frame in frames {
            match frame {
                Frame::OwnerNotice(_) if ended => {}
                frame if frame_fits(&frame) => out.push(frame),
                Frame::OwnerNotice(notice) => {
                    self.subscribed = false;
                    ended = true;
                    out.push(Frame::OwnerNotice(OwnerNotice {
                        sequence: notice.sequence,
                        change: Change::SnapshotRequired,
                    }));
                }
                Frame::OwnerResponse(response) => {
                    let (id, method) = match response {
                        OwnerResponse::Result { id, result } => {
                            (Some(id), frames::method_text(result.method()))
                        }
                        OwnerResponse::Error { id, method, .. } => (id, method),
                    };
                    out.push(frames::error_frame(
                        id,
                        method,
                        OwnerFault::Internal,
                        frames::TOO_LONG_MESSAGE,
                    ));
                }
                frame => out.push(frame),
            }
        }
        out
    }

    /// True from the first successful `subscribe` of the process on.
    pub(crate) fn ever_subscribed(&self) -> bool {
        self.ever_subscribed
    }
}
