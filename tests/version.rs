//! Version, changelog, and forge policy integration tests.

use rabun_git::cli::VersionCommands;
use rabun_git::git;
use rabun_git::release;

async fn git_repo(root: &std::path::Path) {
    git::git_global(&["init", "-b", "master", &root.to_string_lossy()])
        .await
        .unwrap();
    git::git(root, &["config", "user.email", "t@t"])
        .await
        .unwrap();
    git::git(root, &["config", "user.name", "t"]).await.unwrap();
}

#[tokio::test]
async fn hook_install_and_check() {
    if !git::git_on_path() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    git_repo(root).await;
    git::git(root, &["add", "."]).await.unwrap();
    git::git(root, &["commit", "-m", "feat: start"])
        .await
        .unwrap();

    let out = release::run_in(
        root,
        VersionCommands::Hook {
            command: rabun_git::cli::VersionHookCommands::Install,
        },
    )
    .await
    .unwrap();
    assert!(out.contains("commit-msg"));
    let hook = git::git_path(root, "hooks/commit-msg").await.unwrap();
    assert!(hook.exists());
    let script = std::fs::read_to_string(&hook).unwrap();
    assert!(script.contains("hook commit-msg"));

    let check = release::run_in(root, VersionCommands::Check { range: None })
        .await
        .unwrap();
    assert!(check.starts_with("ok:"));
}

#[tokio::test]
async fn enforce_commits_and_tag_manifests() {
    if !git::git_on_path() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".rabun")).unwrap();
    std::fs::write(
        root.join(".rabun/version.toml"),
        "[version]\ntag_prefix = \"v\"\nchangelog = \"CHANGELOG.md\"\n\n[version.enforce]\ncommits = true\ntags = true\nmanifests = true\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    git_repo(root).await;
    git::git(root, &["add", "."]).await.unwrap();
    git::git(root, &["commit", "-m", "feat: start"])
        .await
        .unwrap();
    git::git(root, &["tag", "-a", "v1.0.0", "-m", "v1.0.0"])
        .await
        .unwrap();

    git::git(root, &["commit", "--allow-empty", "-m", "not conventional"])
        .await
        .unwrap();
    let old = git::git_stdout(root, &["rev-parse", "HEAD~1"])
        .await
        .unwrap();
    let new = git::git_stdout(root, &["rev-parse", "HEAD"]).await.unwrap();
    let err = release::enforce_push(root, "refs/heads/master", &old, &new)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Conventional Commits"));

    git::git(root, &["reset", "--hard", "HEAD~1"])
        .await
        .unwrap();
    git::git(root, &["commit", "--allow-empty", "-m", "fix: nil"])
        .await
        .unwrap();
    let old = git::git_stdout(root, &["rev-parse", "HEAD~1"])
        .await
        .unwrap();
    let new = git::git_stdout(root, &["rev-parse", "HEAD"]).await.unwrap();
    release::enforce_push(root, "refs/heads/master", &old, &new)
        .await
        .unwrap();

    let head = git::git_stdout(root, &["rev-parse", "HEAD"]).await.unwrap();
    git::git(root, &["tag", "-a", "v1.0.1", "-m", "v1.0.1"])
        .await
        .unwrap();
    let tag = git::git_stdout(root, &["rev-parse", "v1.0.1"])
        .await
        .unwrap();
    let err = release::enforce_push(root, "refs/tags/v1.0.1", "0", &tag)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not match") || err.to_string().contains("1.0.0"));
    let _ = head;
}
