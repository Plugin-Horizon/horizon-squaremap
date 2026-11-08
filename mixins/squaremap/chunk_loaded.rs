fn sm_chunk_loaded(level: &str, pos: [i32; 2], fresh: bool) {
    crate::game::squaremap_glue::chunk_loaded(level, pos, fresh);
}
