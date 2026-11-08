//! `session.*`: undo, redo and the state of the session.
use crate::{
    command::{Cmd, CommandSpec, Context, Kind, command},
    engine::{Empty, EngineError, HistoryRequest, SessionStatus},
    session::Session,
};

pub const SESSION_UNDO: Cmd<HistoryRequest, SessionStatus> = Cmd::new("session.undo");
pub const SESSION_REDO: Cmd<HistoryRequest, SessionStatus> = Cmd::new("session.redo");
pub const SESSION_STATUS: Cmd<Empty, SessionStatus> = Cmd::new("session.status");

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command(
            SESSION_UNDO,
            "直前の文書の変更を取り消す",
            Kind::Mutation,
            session_undo,
        ),
        command(
            SESSION_REDO,
            "取り消した変更をやり直す",
            Kind::Mutation,
            session_redo,
        ),
        command(
            SESSION_STATUS,
            "セッションの版番号・保存先・履歴の状態を返す",
            Kind::Query,
            session_status,
        ),
    ]
}

fn session_undo(
    session: &mut Session,
    r: HistoryRequest,
    _: &mut Context<'_>,
) -> Result<SessionStatus, EngineError> {
    session.undo(r.expected_revision)?;
    Ok(session.status())
}

fn session_redo(
    session: &mut Session,
    r: HistoryRequest,
    _: &mut Context<'_>,
) -> Result<SessionStatus, EngineError> {
    session.redo(r.expected_revision)?;
    Ok(session.status())
}

fn session_status(
    session: &mut Session,
    _: Empty,
    _: &mut Context<'_>,
) -> Result<SessionStatus, EngineError> {
    Ok(session.status())
}
