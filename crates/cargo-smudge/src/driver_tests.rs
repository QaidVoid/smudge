use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn reads_the_rustc_a_cargo_config_selects() {
    let dir = std::env::temp_dir().join("smudge-config-test");
    std::fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "# the compiler cargo runs\n[build]\nrustc = \"/opt/a/rustc\" # trailing\n\n\
         [target.'cfg(unix)'.rustflags]\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_string(&path, "build", "rustc").as_deref(),
        Some("/opt/a/rustc")
    );
    std::fs::write(&path, "build.rustc = \"./rustc-b\"\n")
        .expect("the config could not be written");
    assert_eq!(
        config_string(&path, "build", "rustc").as_deref(),
        Some("./rustc-b")
    );
    std::fs::write(&path, "[build]\ntarget = \"x\"\n").expect("the config could not be written");
    assert_eq!(config_string(&path, "build", "rustc"), None);
}

#[test]
fn reads_literal_strings_and_the_legacy_config() {
    let base = std::env::temp_dir().join("smudge-config-literal");
    let dir = base.join("app");
    let cargo = dir.join(".cargo");
    std::fs::create_dir_all(&cargo).expect("the scratch directory could not be made");
    let _ = std::fs::remove_file(cargo.join("config"));
    // A literal string carries the path as it is written, which a
    // file name with quotes of its own still spells.
    std::fs::write(
        cargo.join("config.toml"),
        "[build]\nrustc = './comps/rustc'\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("./comps/rustc"), dir.clone()))
    );
    // The legacy file counts over the TOML one when both stand, the
    // way cargo reads them, and the file that loses is not read.
    std::fs::write(cargo.join("config"), "[build]\nrustc = \"/opt/b/rustc\"\n")
        .expect("the legacy config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("/opt/b/rustc"), dir.clone()))
    );
    std::fs::remove_file(cargo.join("config")).expect("the legacy config could not go");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("./comps/rustc"), dir))
    );
}

#[test]
fn reads_a_dotted_key_whatever_whitespace_it_carries() {
    let dir = std::env::temp_dir().join("smudge-config-dotted");
    let cargo = dir.join(".cargo");
    std::fs::create_dir_all(&cargo).expect("the scratch directory could not be made");
    let _ = std::fs::remove_file(cargo.join("config"));
    // A dotted key at the top level may carry whitespace around its
    // separators, and a header may carry whitespace inside its
    // brackets.
    std::fs::write(
        cargo.join("config.toml"),
        "[ build ]\nrustc = '/opt/a/rustc'\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("/opt/a/rustc"), dir.clone()))
    );
    std::fs::write(
        cargo.join("config.toml"),
        "build . rustc = '/opt/c/rustc'\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("/opt/c/rustc"), dir.clone()))
    );
    // A key under another table stays out, whatever its own name is.
    std::fs::write(
        cargo.join("config.toml"),
        "[build.other]\nrustc = '/opt/b/rustc'\n",
    )
    .expect("the config could not be written");
    assert_eq!(config_rustc(&dir), None);
    // A dotted key under a header names a subtable of that header,
    // the way TOML scopes it: `build.rustc` under `[build]` is
    // `build.build.rustc`, which cargo leaves unused, and the bare
    // key beside it is the compiler.
    std::fs::write(
        cargo.join("config.toml"),
        "[build]\nbuild.rustc = '/decoy/rustc'\nrustc = '/actual/rustc'\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("/actual/rustc"), dir.clone()))
    );
    std::fs::write(
        cargo.join("config.toml"),
        "[build]\nbuild.rustc = '/decoy/rustc'\n",
    )
    .expect("the config could not be written");
    assert_eq!(config_rustc(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reads_the_escapes_a_basic_string_defines() {
    let dir = std::env::temp_dir().join("smudge-config-escapes");
    let cargo = dir.join(".cargo");
    std::fs::create_dir_all(&cargo).expect("the scratch directory could not be made");
    let _ = std::fs::remove_file(cargo.join("config"));
    std::fs::write(
        cargo.join("config.toml"),
        "[build]\nrustc = \"\\u0072ustc\"\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("rustc"), dir.clone()))
    );
    std::fs::write(cargo.join("config.toml"), "[build]\nrustc = \"a\\tb\"\n")
        .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("a\tb"), dir.clone()))
    );
    // The short hexadecimal and the escape-character escapes the
    // TOML cargo reads accepts spell ordinary names too.
    std::fs::write(
        cargo.join("config.toml"),
        "[build]\nrustc = \"/opt/\\x61/rustc\"\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("/opt/a/rustc"), dir.clone()))
    );
    std::fs::write(
        cargo.join("config.toml"),
        "[build]\nrustc = \"/opt/\\e/rustc\"\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_rustc(&dir),
        Some((String::from("/opt/\u{001b}/rustc"), dir.clone()))
    );
    // An escape of the wrong width reads as nothing, which the file
    // would not survive under cargo either.
    std::fs::write(cargo.join("config.toml"), "[build]\nrustc = \"a\\xb\"\n")
        .expect("the config could not be written");
    assert_eq!(config_rustc(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reads_the_search_path_the_env_table_sets() {
    let dir = std::env::temp_dir().join("smudge-config-envpath");
    let cargo = dir.join(".cargo");
    std::fs::create_dir_all(&cargo).expect("the scratch directory could not be made");
    let _ = std::fs::remove_file(cargo.join("config"));
    std::fs::write(
        cargo.join("config.toml"),
        "[env.PATH]\nvalue = '/compilers/a'\nforce = true\n",
    )
    .expect("the config could not be written");
    let found = config_env_path(&cargo.join("config.toml"));
    assert_eq!(
        found.and_then(|found| found.value),
        Some(String::from("/compilers/a"))
    );
    std::fs::write(
        cargo.join("config.toml"),
        "[env.PATH]\nvalue = '/compilers/a'\nforce = true\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_env_path(&cargo.join("config.toml")).and_then(|found| found.force),
        Some(true)
    );
    // The bare key under `[env]` fills the value alone, and a
    // `relative` value joins the directory the `.cargo` sits in.
    std::fs::write(
        cargo.join("config.toml"),
        "[env]\nPATH = \"/compilers/b\"\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        config_env_path(&cargo.join("config.toml")).and_then(|found| found.value),
        Some(String::from("/compilers/b"))
    );
    assert_eq!(
        config_env_path(&cargo.join("config.toml")).and_then(|found| found.force),
        None
    );
    std::fs::write(
        cargo.join("config.toml"),
        "[env.PATH]\nvalue = 'here'\nrelative = true\nforce = true\n",
    )
    .expect("the config could not be written");
    let search = effective_search_path(&dir);
    assert_eq!(search, std::ffi::OsString::from(dir.join("here")));
    // A dotted key at the top level names the table's fields, the
    // way TOML scopes it, and a dotted key under `[env]` names the
    // table's own key.
    std::fs::write(
        cargo.join("config.toml"),
        "env.PATH.value = '/compilers/c'\nenv.PATH.force = true\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        effective_search_path(&dir),
        std::ffi::OsString::from("/compilers/c")
    );
    std::fs::write(
        cargo.join("config.toml"),
        "[env]\nPATH.value = '/compilers/d'\nPATH.force = true\n",
    )
    .expect("the config could not be written");
    assert_eq!(
        effective_search_path(&dir),
        std::ffi::OsString::from("/compilers/d")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn merges_the_env_path_fields_across_configurations() {
    let parent = std::env::temp_dir().join("smudge-config-envmerge");
    let child = parent.join("app").join(".cargo");
    std::fs::create_dir_all(&child).expect("the scratch directory could not be made");
    std::fs::create_dir_all(parent.join(".cargo"))
        .expect("the scratch directory could not be made");
    std::fs::write(
        parent.join(".cargo").join("config.toml"),
        "[env.PATH]\nvalue = '/compilers/parent'\nforce = true\n",
    )
    .expect("the parent config could not be written");
    std::fs::write(
        child.join("config.toml"),
        "[env.PATH]\nvalue = '/compilers/child'\n",
    )
    .expect("the child config could not be written");
    let search = effective_search_path(&child);
    assert_eq!(
        search,
        std::ffi::OsString::from("/compilers/child"),
        "the nearer file's value wins and the parent's force stands"
    );
    // A nearer force of its own wins over the parent's: the merged
    // table does not override the process's own PATH then.
    std::fs::write(
        child.join("config.toml"),
        "[env.PATH]\nvalue = '/compilers/child'\nforce = false\n",
    )
    .expect("the child config could not be written");
    let search = effective_search_path(&child);
    assert_eq!(
        search,
        std::env::var_os("PATH").unwrap_or_default(),
        "the nearer false answers over the parent's true"
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn checks_the_execute_permission_the_lookup_checks() {
    let dir = std::env::temp_dir().join("smudge-exec-test");
    std::fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let runnable = dir.join("runnable");
    let _ = std::fs::remove_file(&runnable);
    std::fs::write(&runnable, "#!/bin/sh\n").expect("the script could not be written");
    let mut permissions = std::fs::metadata(&runnable)
        .expect("the script could not be measured")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&runnable, permissions)
        .expect("the script could not be made runnable");
    assert!(executable(&runnable));
    // One execute bit for a class that is not this process's own
    // leaves the file unreachable, the way the lookup skips it.
    let blocked = dir.join("blocked");
    let _ = std::fs::remove_file(&blocked);
    std::fs::write(&blocked, "#!/bin/sh\n").expect("the script could not be written");
    let mut permissions = std::fs::metadata(&blocked)
        .expect("the script could not be measured")
        .permissions();
    permissions.set_mode(0o001);
    std::fs::set_permissions(&blocked, permissions).expect("the script could not be made blocked");
    assert!(!executable(&blocked));
}

#[test]
fn watches_a_rename_that_keeps_the_modification_time() {
    let base = std::env::temp_dir().join("smudge-link-rename");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("src");
    std::fs::create_dir_all(&src).expect("the source directory could not be made");
    std::fs::write(src.join("old.rs"), "old").expect("the old file could not be written");
    std::fs::write(src.join("new.rs"), "new").expect("the new file could not be written");
    std::os::unix::fs::symlink("old.rs", src.join("alias.rs"))
        .expect("the alias could not be made");
    // The replacement is prepared before the watermark is taken, so
    // both its own time and its target's predate it, and only the
    // change time a rename stamps can carry the swap.
    std::os::unix::fs::symlink("new.rs", src.join("fresh.rs"))
        .expect("the replacement could not be made");
    let alias = src.join("alias.rs");
    let before = changed_nanos(&alias).expect("the alias answers its time");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::rename(src.join("fresh.rs"), &alias).expect("the replacement could not be moved over");
    let after = changed_nanos(&alias).expect("the alias answers its time");
    assert!(
        after != before,
        "a rename over the alias left no trace: {before} -> {after}"
    );
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn watches_a_link_an_inner_retarget_moves() {
    let base = std::env::temp_dir().join("smudge-link-chain");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("src");
    let old = base.join("data").join("old");
    let new = base.join("data").join("new");
    std::fs::create_dir_all(&src).expect("the source directory could not be made");
    std::fs::create_dir_all(&old).expect("the old directory could not be made");
    std::fs::create_dir_all(&new).expect("the new directory could not be made");
    std::fs::write(old.join("tag.rs"), "old").expect("the old file could not be written");
    std::fs::write(new.join("tag.rs"), "new").expect("the new file could not be written");
    std::os::unix::fs::symlink("old/tag.rs", base.join("data").join("hop"))
        .expect("the inner link could not be made");
    std::os::unix::fs::symlink("../data/hop", src.join("alias.rs"))
        .expect("the alias could not be made");
    let alias = src.join("alias.rs");
    let before = changed_nanos(&alias).expect("the alias answers its time");
    // The hop is no component of the alias's own name, so only the
    // chain its resolution walks can carry the retarget's time.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::remove_file(base.join("data").join("hop")).expect("the inner link could not go");
    std::os::unix::fs::symlink("new/tag.rs", base.join("data").join("hop"))
        .expect("the inner link could not be retargeted");
    let after = changed_nanos(&alias).expect("the alias answers its time");
    assert!(
        after != before,
        "a retarget of the inner link left no trace: {before} -> {after}"
    );
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn watches_a_link_a_retarget_of_its_directory_moves() {
    let base = std::env::temp_dir().join("smudge-link-dir");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("src");
    std::fs::create_dir_all(&src).expect("the source directory could not be made");
    std::fs::create_dir_all(base.join("old")).expect("the old directory could not be made");
    std::fs::create_dir_all(base.join("new")).expect("the new directory could not be made");
    std::fs::write(base.join("old").join("tag.rs"), "old")
        .expect("the old file could not be written");
    std::fs::write(base.join("new").join("tag.rs"), "new")
        .expect("the new file could not be written");
    std::os::unix::fs::symlink("old", base.join("data"))
        .expect("the directory link could not be made");
    std::os::unix::fs::symlink("../data/tag.rs", src.join("alias.rs"))
        .expect("the alias could not be made");
    let alias = src.join("alias.rs");
    let before = changed_nanos(&alias).expect("the alias answers its time");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::remove_file(base.join("data")).expect("the link could not go");
    std::os::unix::fs::symlink("new", base.join("data"))
        .expect("the directory link could not be retargeted");
    let after = changed_nanos(&alias).expect("the alias answers its time");
    assert!(
        after != before,
        "a retarget of the directory link left no trace: {before} -> {after}"
    );
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn watches_a_retarget_that_a_future_stamp_hides() {
    let base = std::env::temp_dir().join("smudge-link-future");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("src");
    let data = base.join("data");
    std::fs::create_dir_all(&src).expect("the source directory could not be made");
    std::fs::create_dir_all(&data).expect("the data directory could not be made");
    std::fs::write(data.join("old.txt"), "old").expect("the old file could not be written");
    std::fs::write(data.join("new.txt"), "new").expect("the new file could not be written");
    // Both targets stand an hour ahead, so their times answer for
    // nothing a newest-of comparison could still move: only a fold
    // of the separate states carries the link's own move.
    let ahead = std::time::SystemTime::now() + Duration::from_secs(3600);
    for name in ["old.txt", "new.txt"] {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(data.join(name))
            .expect("the target could not be opened");
        file.set_modified(ahead)
            .expect("the stamp could not be moved ahead");
    }
    std::os::unix::fs::symlink("../data/old.txt", src.join("alias.txt"))
        .expect("the alias could not be made");
    let alias = src.join("alias.txt");
    let before = changed_nanos(&alias).expect("the alias answers its fold");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::remove_file(&alias).expect("the alias could not go");
    std::os::unix::fs::symlink("../data/new.txt", &alias)
        .expect("the alias could not be retargeted");
    let after = changed_nanos(&alias).expect("the alias answers its fold");
    assert!(
        after != before,
        "a retarget the future stamps hid left no trace: {before} -> {after}"
    );
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn reads_the_declarations_a_chatty_output_carries() {
    let package = Path::new("/work/tools");
    // A run's chatter may carry bytes no encoding names, which
    // cargo reads past; the directives beside them survive the
    // lossy reading, and the broken byte neither eats the line it
    // sits in nor the ones after it.
    let text = String::from_utf8_lossy(&[
        b'c', b'a', b'r', b'g', b'o', b':', b's', b'a', b'y', b'=', 0xff, b'\n', b'c', b'a', b'r',
        b'g', b'o', b':', b'r', b'e', b'r', b'u', b'n', b'-', b'i', b'f', b'-', b'c', b'h', b'a',
        b'n', b'g', b'e', b'd', b'=', b'p', b'i', b'e', b'.', b't', b'x', b't', b'\n',
    ])
    .into_owned();
    let (files, dirs) = declared_inputs_of(&text, package);
    assert_eq!(files, vec![PathBuf::from("/work/tools/pie.txt")]);
    assert!(dirs.is_empty(), "a file directive answers no directory");
}

#[test]
fn reads_the_declarations_an_indent_carries() {
    let package = Path::new("/work/tools");
    // Cargo trims the line before it reads the directive, so an
    // indent answers all the same, while the value keeps whatever
    // follows the equals sign as it is written. A spelling that
    // names nothing yet counts as a file, since only what exists
    // now can say it is a directory.
    let (files, dirs) = declared_inputs_of(
        "  cargo:rerun-if-changed=pie.txt\n\tcargo::rerun-if-changed=assets\n",
        package,
    );
    assert_eq!(
        files,
        vec![
            PathBuf::from("/work/tools/pie.txt"),
            PathBuf::from("/work/tools/assets")
        ]
    );
    assert!(dirs.is_empty());
}

#[test]
fn reads_the_keys_quotes_name() {
    // A quoted key names the same key as the bare one, and a dot
    // inside the quotes does not split.
    assert_eq!(
        dotted_parts("build.rustc"),
        vec![String::from("build"), String::from("rustc")]
    );
    assert_eq!(
        dotted_parts("package.\"build\""),
        vec![String::from("package"), String::from("build")]
    );
    assert_eq!(dotted_parts("'build'"), vec![String::from("build")]);
    assert_eq!(dotted_parts("\"a.b\""), vec![String::from("a.b")]);
    assert_eq!(
        dotted_parts("build . rustc"),
        vec![String::from("build"), String::from("rustc")]
    );
}

#[test]
fn sees_an_edit_after_a_state_fold_grew_past_the_clock() {
    let base = std::env::temp_dir().join("smudge-fold-edit");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("the scratch could not be made");
    let file = base.join("lib.rs");
    std::fs::write(&file, "pub const A: u32 = 1;\n")
        .expect("the scratch source could not be written");
    let watch = vec![base.clone()];
    let (_, times) = scan_sources(&watch, &HashMap::new());
    assert!(times.contains_key(&file), "the walk missed the file");
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(&file, "pub const A: u32 = 2;\n").expect("the edit could not be written");
    let (fresh, _) = scan_sources(&watch, &times);
    assert!(
        fresh.iter().any(|(_, path)| path == &file),
        "the folded state of an edit hid behind the clock clamp"
    );
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn reads_the_manifest_script_keys_whatever_form_they_take() {
    let base = std::env::temp_dir().join("smudge-script-keys");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("the scratch could not be made");
    let manifest = base.join("Cargo.toml");
    std::fs::write(base.join("build.rs"), "fn main() {}\n")
        .expect("the automatic script could not be written");
    // No key answers by the bare automatic `build.rs`, which stands.
    std::fs::write(&manifest, "[package]\nname = \"a\"\n").unwrap();
    assert!(has_build_script(&base));
    // `build = false` keeps cargo from treating the file as a script,
    // however the key is spelled.
    std::fs::write(&manifest, "[package]\nbuild = false\n").unwrap();
    assert!(!has_build_script(&base));
    std::fs::write(&manifest, "package.build = false\n").unwrap();
    assert!(!has_build_script(&base));
    // A named script answers by the file it names.
    std::fs::write(&manifest, "[package]\nbuild = \"codegen.rs\"\n").unwrap();
    assert!(!has_build_script(&base));
    std::fs::write(base.join("codegen.rs"), "fn main() {}\n").unwrap();
    assert!(has_build_script(&base));
    // A quoted key names the same key as the bare one, in either
    // string kind.
    std::fs::write(&manifest, "[package]\n\"build\" = \"codegen.rs\"\n").unwrap();
    assert!(has_build_script(&base));
    std::fs::write(&manifest, "[package]\n'build' = false\n").unwrap();
    assert!(!has_build_script(&base));
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn chooses_the_named_packages_bin() {
    let base = std::env::temp_dir().join("smudge-default-bin");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("a").join("src")).expect("the scratch could not be made");
    std::fs::create_dir_all(base.join("b").join("src")).expect("the scratch could not be made");
    std::fs::write(
        base.join("Cargo.toml"),
        "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        base.join("a").join("Cargo.toml"),
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(base.join("a").join("src").join("lib.rs"), "").unwrap();
    std::fs::write(
        base.join("b").join("Cargo.toml"),
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(base.join("b").join("src").join("main.rs"), "fn main() {}\n").unwrap();
    // The bin answers for the package the request names, which the
    // directory of the invocation only carries when none was named.
    assert_eq!(
        default_bin(&base.join("a"), Some("b")).as_deref(),
        Some("b")
    );
    // A package specification with a version names the same package.
    assert_eq!(
        default_bin(&base.join("a"), Some("b@0.1.0")).as_deref(),
        Some("b")
    );
    assert_eq!(default_bin(&base.join("a"), None), None);
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}

#[test]
fn walks_an_alias_spelling_whatever_its_target_stood() {
    use std::os::unix::fs::symlink;

    let base = std::env::temp_dir().join(format!("smudge-walk-{}", std::process::id()));
    let src = base.join("src");
    std::fs::create_dir_all(&src).expect("the scratch could not be made");
    std::fs::write(src.join("lib.rs"), "").unwrap();
    for named in ["a", "b"] {
        let dir = src.join(named);
        std::fs::create_dir_all(&dir).expect("the scratch could not be made");
        std::fs::write(dir.join("tag.rs"), "").unwrap();
    }
    symlink("a", src.join("link")).expect("the alias could not be made");
    // A loop the walk must refuse without spinning.
    symlink(&base, src.join("loop")).expect("the loop could not be made");

    let spellings = |root: &Path| {
        let mut found = Vec::new();
        walk(root, &mut |path| found.push(path.to_path_buf()));
        found
    };
    let names = |found: &[PathBuf]| {
        let mut named: Vec<String> = found
            .iter()
            .filter_map(|path| path.strip_prefix(&base).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        named.sort();
        named
    };
    let first = names(&spellings(&src));
    assert!(
        first.contains(&String::from("src/link/tag.rs")),
        "the alias spelling did not answer: {first:?}"
    );
    assert!(
        !first.iter().any(|spelled| spelled.starts_with("src/loop/")),
        "the loop was walked: {first:?}"
    );

    // The retarget moves what the alias names, which the alias's
    // own spelling must answer for however the directory order
    // visited its targets before.
    let _ = std::fs::remove_file(src.join("link"));
    symlink("b", src.join("link")).expect("the retarget could not be made");
    let second = names(&spellings(&src));
    assert!(
        second.contains(&String::from("src/link/tag.rs")),
        "the retargeted alias stopped answering: {second:?}"
    );
    std::fs::remove_dir_all(&base).expect("the scratch could not go");
}
