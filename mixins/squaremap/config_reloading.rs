fn sm_config_reloading(file: &crate::game::native_configs::NativeConfigFile) {
    if file.mod_id == "squaremap" {
        horizon_squaremap::config_loaded(&file.text);
    }
}
