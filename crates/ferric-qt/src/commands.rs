//! `CommandBus` skeleton: every UI action routes through one string-keyed
//! entrypoint, keyed by the ids in `tasks/cxx-qt/catalog/actions.json`
//! (enforced by FQT-018's wire-audit CI stub). No action is wired yet —
//! handlers land alongside each action as it ships (FQT-021 downloads,
//! FQT-032 torrents); until then every id is a deliberate, honest error
//! rather than a silent no-op.

pub fn dispatch(action_id: &str) -> Result<(), String> {
    Err(format!("action '{action_id}' not wired yet"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwired_action_returns_error_not_panic() {
        assert!(dispatch("dl.pause").is_err());
    }
}
