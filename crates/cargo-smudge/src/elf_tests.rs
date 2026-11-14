use std::path::{Path, PathBuf};

use super::sysroot_symbols;

/// The standard library rlib of the compiler on the path.
fn std_rlib() -> Option<PathBuf> {
    let output = std::process::Command::new("rustc")
        .args(["--print", "target-libdir"])
        .output()
        .ok()?;
    let dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libstd-") && name.ends_with(".rlib"))
        })
}

/// Copies `rlib` to `to`, making its directory first.
fn place(rlib: &Path, to: &Path) {
    std::fs::create_dir_all(to.parent().expect("the path has a directory"))
        .expect("the directory could not be made");
    std::fs::copy(rlib, to).expect("the rlib could not be copied");
}

#[test]
fn only_the_sysroots_own_libraries_count_as_the_standard_librarys() {
    let Some(rlib) = std_rlib() else {
        eprintln!("skipping: the compiler on the path names no standard library rlib");
        return;
    };
    // A project built under the sysroot, as under `/usr` with a packaged
    // compiler, holds rlibs under it that are not the standard library's.
    let sysroot = std::env::temp_dir().join("smudge-sysroot-test");
    let _ = std::fs::remove_dir_all(&sysroot);
    let library = sysroot.join("lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd.rlib");
    let project = sysroot.join("src/app/target/debug/deps/libapp.rlib");
    place(&rlib, &library);
    place(&rlib, &project);

    let from_project = sysroot_symbols(&[project.display().to_string()], &sysroot)
        .expect("the project's rlib is read");
    let from_library = sysroot_symbols(&[library.display().to_string()], &sysroot)
        .expect("the sysroot's rlib is read");

    assert!(
        from_project.is_empty(),
        "a project's rlib counted as the standard library's"
    );
    assert!(
        !from_library.is_empty(),
        "the sysroot's own rlib named no symbols"
    );

    // A sysroot whose `lib` is a link elsewhere still owns what lies there.
    let linked = std::env::temp_dir().join("smudge-sysroot-linked-test");
    let _ = std::fs::remove_dir_all(&linked);
    std::fs::create_dir_all(&linked).expect("the linked sysroot could not be made");
    std::os::unix::fs::symlink(sysroot.join("lib"), linked.join("lib"))
        .expect("the link could not be made");
    let through_link = linked.join("lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd.rlib");
    let from_link = sysroot_symbols(&[through_link.display().to_string()], &linked)
        .expect("the linked rlib is read");
    assert_eq!(
        from_link, from_library,
        "a linked library directory lost its rlibs"
    );
    let _ = std::fs::remove_dir_all(&linked);

    // A compiler that keeps its libraries under `lib64` reads them there.
    let wide = std::env::temp_dir().join("smudge-sysroot-lib64-test");
    let _ = std::fs::remove_dir_all(&wide);
    let wide_library = wide.join("lib64/rustlib/x86_64-unknown-linux-gnu/lib/libstd.rlib");
    place(&rlib, &wide_library);
    let from_wide = sysroot_symbols(&[wide_library.display().to_string()], &wide)
        .expect("the lib64 rlib is read");
    assert_eq!(from_wide, from_library, "a lib64 sysroot lost its rlibs");
    let _ = std::fs::remove_dir_all(&wide);
    let _ = std::fs::remove_dir_all(&sysroot);
}

#[test]
fn a_relative_sysroot_names_no_standard_library() {
    let Some(rlib) = std_rlib() else {
        eprintln!("skipping: the compiler on the path names no standard library rlib");
        return;
    };
    // A relative sysroot would resolve against the working directory. The
    // rlib sits in the temp dir, named relative to wherever the test runs,
    // so the checkout stays untouched while the path still resolves.
    let root = std::env::temp_dir().join("smudge-relative-sysroot-test");
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("lib/rustlib/x86_64-unknown-linux-gnu/lib/libapp.rlib");
    place(&rlib, &project);
    let depth = std::env::current_dir()
        .expect("the working directory is known")
        .components()
        .count();
    let relative = PathBuf::from("../".repeat(depth))
        .join(root.strip_prefix("/").expect("the temp dir is absolute"));

    let found = sysroot_symbols(&[project.display().to_string()], &relative);

    let _ = std::fs::remove_dir_all(&root);
    assert!(
        found.expect("the call succeeds").is_empty(),
        "a relative sysroot counted the rlibs it resolves to"
    );
}
