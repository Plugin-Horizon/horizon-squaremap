fn sm_server_ticked(game: &mut crate::game::Game<'_>, phase: horizon_client::mod_api::ServerTickPhase) {
    crate::game::squaremap_glue::server_ticked(game, phase);
}
