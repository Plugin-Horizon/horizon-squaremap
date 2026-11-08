fn sm_chunk_unloaded(level: &str, pos: [i32; 2]) {
    crate::game::squaremap_glue::chunk_unloaded(level, pos);
}
