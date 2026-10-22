use ulanzi_core::template::{TemplateSource, instantiate, list_in};

#[test]
fn builtins_parse_and_are_generic() {
    let d = tempfile::tempdir().expect("tmp");
    let all = list_in(Some(d.path()));
    let builtins: Vec<_> = all
        .iter()
        .filter(|t| matches!(t.source, TemplateSource::BuiltIn(_)))
        .collect();
    assert!(builtins.len() >= 4, "{}", builtins.len());
    for t in builtins {
        let b = instantiate(t).expect("instantiate");
        assert_eq!(b.profiles.len(), 1);
        let p = &b.profiles[0].profile;
        assert!(!p.pages[0].keys.is_empty(), "{} has keys", t.name);
        let text = format!(
            "{} {} {}",
            t.name,
            t.description,
            toml::to_string(p).expect("toml")
        )
        .to_lowercase();
        assert!(!text.is_empty(), "{} must have content", t.name);
        let lib = ulanzi_core::render::IconLibrary::bundled();
        for k in p.pages[0].keys.values() {
            if let ulanzi_core::model::ImageSource::Icon { set, name } = &k.look.source {
                assert!(lib.svg(*set, name).is_some(), "{}: icon {name} missing", t.name);
            }
        }
        // A template survives the trip through a file.
        let again =
            ulanzi_core::bundle::read(&ulanzi_core::bundle::to_bytes(&b).expect("bytes")).expect("read");
        assert_eq!(again.profiles[0].profile, *p);
    }
}

#[test]
fn user_templates_are_listed() {
    let d = tempfile::tempdir().expect("tmp");
    let store = ulanzi_core::store::Store::at(d.path().join("c"), d.path().join("s"), d.path().join("k"));
    let p = ulanzi_core::model::Profile::new_default("My Layout", ulanzi_core::model::Model::D200X);
    store.save_profile(&p).expect("save");
    let dir = d.path().join("templates");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(
        dir.join("my-layout.ulanzideck"),
        ulanzi_core::bundle::export(&store, &[p.id], false).expect("export"),
    )
    .expect("write");
    let all = list_in(Some(&dir));
    let mine = all.iter().find(|t| t.name == "My Layout").expect("listed");
    assert!(matches!(mine.source, TemplateSource::File(_)));
}

#[test]
fn instantiate_gives_fresh_ids() {
    let all = list_in(None);
    let a = instantiate(&all[0]).expect("a").profiles[0].profile.id;
    let b = instantiate(&all[0]).expect("b").profiles[0].profile.id;
    assert_ne!(a, b);
}
