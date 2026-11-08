fn sm_world_phase(game: &mut crate::game::Game<'_>, world_phase: horizon_client::mod_api::WorldPhase) {
    crate::game::squaremap_glue::world_phase(game, world_phase);
}
