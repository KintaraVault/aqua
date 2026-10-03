fn main() {
    let cfg = slint_build::CompilerConfiguration::new()
        .with_style("cupertino-light".into())
        // translations/<lang>/LC_MESSAGES/aqua-ui.po, compiled in; the language follows the
        // system locale (or AQUA_LANG, see `aqua_ui::init_translations`).
        .with_bundled_translations("translations")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    println!("cargo:rerun-if-changed=translations");
    slint_build::compile_with_config("ui/app.slint", cfg).expect("slint compile");
}
