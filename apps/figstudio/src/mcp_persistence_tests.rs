//! Persistence must remember user intent, but never a running server's bearer token.
use eframe::App as _;

use super::*;

use crate::ui_tests::Memory;

#[test]
fn a_failed_bind_preserves_saved_access_intent_and_retries_after_restart() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let busy = TcpListener::bind(ANY_PORT).expect("occupied test port");
    let mut app = crate::App::new(&cc, Vec::new());
    app.mcp = host(vec![busy.local_addr().expect("test address")]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.mcp.sync(&ctx, &[], 0);
        if app.mcp.frame(&mut [], &mut 0).is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "failed bind was not reported");
        std::thread::yield_now();
    }
    let mut memory = Memory::default();
    app.save(&mut memory);
    let saved: crate::UiState =
        eframe::get_value(&memory, crate::UI_STATE_KEY).expect("saved preferences");
    assert!(
        saved.prefs.agents.on,
        "a bind failure must not change the user's choice"
    );
    for _ in 0..2 {
        app.mcp.sync(&ctx, &[], 0);
        assert_eq!(
            app.mcp.frame(&mut [], &mut 0),
            None,
            "no per-frame retry loop"
        );
        assert!(app.mcp.running.is_none());
    }
    let mut cc = eframe::CreationContext::_new_kittest(ctx.clone());
    cc.storage = Some(&memory);
    let mut restarted = crate::App::new(&cc, Vec::new());
    assert!(restarted.mcp.on);
    restarted.mcp.addrs = vec![ANY_PORT.parse().expect("ephemeral loopback")];
    wait_started(&mut restarted.mcp, &ctx, &mut []);
    restarted.mcp.stop();
}

#[test]
fn a_live_minted_remote_bearer_token_is_not_saved() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = crate::App::new(&cc, Vec::new());
    // Mint real remote access rules, but bind the test listener only on loopback.
    let remote = "192.0.2.1:3845".parse().expect("documentation address");
    let access = HttpAccess::for_bind(remote, &[], false).expect("remote access rules");
    let token = access.token().expect("minted bearer token").to_owned();
    app.mcp = McpHost {
        on: true,
        remote: true,
        remote_addr: remote.to_string(),
        running: Some(start(
            &ctx,
            vec![ANY_PORT.parse().expect("loopback")],
            access,
        )),
        ..Default::default()
    };
    wait_started(&mut app.mcp, &ctx, &mut []);
    assert!(
        app.mcp
            .access()
            .expect("access controls")
            .endpoint
            .and_then(|(_, token)| token)
            .is_some(),
        "the live endpoint actually has a token"
    );
    let mut memory = Memory::default();
    app.save(&mut memory);
    app.mcp.stop();
    assert!(
        memory.0.values().all(|value| !value.contains(&token)),
        "a live credential leaked into saved state"
    );
    let saved: crate::UiState =
        eframe::get_value(&memory, crate::UI_STATE_KEY).expect("saved preferences");
    assert!(saved.prefs.agents.on && saved.prefs.agents.remote);
}
