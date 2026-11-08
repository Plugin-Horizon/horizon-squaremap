fn sm_block_changed(level: horizon_world::level::Dimension, at: horizon_core::position::BlockPos) {
    crate::game::squaremap_glue::block_changed(level, at.x, at.z);
}
