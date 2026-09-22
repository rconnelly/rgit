//! Integration tests with a real `git` binary.

use rabun_git::acl::{self, Actor, Role};
use rabun_git::git;
use rabun_git::names::RepoName;
use rabun_git::repo;
use rabun_git::request;
use rabun_git::store::Store;
use rabun_git::workflow;

fn git_ok() -> bool {
    git::git_on_path()
}

async fn seed_repo(store: &Store, name: &RepoName, workflow: &str) -> String {
    repo::create(store, &Actor::Operator, name).await.unwrap();
    let bare = store.repo_path(name);
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("src");
    std::fs::create_dir_all(work.join(".rabun/workflows")).unwrap();
    std::fs::write(work.join(".rabun/workflows/ci.yml"), workflow).unwrap();
    std::fs::write(work.join("README.md"), "hi\n").unwrap();
    git::git_global(&["init", "-b", "master", &work.to_string_lossy()])
        .await
        .unwrap();
    git::git(&work, &["config", "user.email", "t@t"])
        .await
        .unwrap();
    git::git(&work, &["config", "user.name", "t"])
        .await
        .unwrap();
    git::git(&work, &["add", "."]).await.unwrap();
    git::git(&work, &["commit", "-m", "init"]).await.unwrap();
    git::git(&work, &["remote", "add", "origin", &bare.to_string_lossy()])
        .await
        .unwrap();
    git::git(&work, &["push", "-u", "origin", "HEAD:master"])
        .await
        .unwrap();
    git::rev_parse(&bare, "HEAD").await.unwrap()
}

#[tokio::test]
async fn init_check_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let toml = tmp.path().join("rabun-git.toml");
    rabun_git::setup::init(Some(&toml)).unwrap();
    assert!(toml.exists());
    assert!(tmp.path().join("data/git/users.yaml").exists());
    assert!(tmp.path().join(".env.example").exists());
}

#[tokio::test]
async fn acl_deny_read() {
    if !git_ok() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path());
    store.ensure_layout().unwrap();
    store.add_user("alice", true).unwrap();
    store.add_user("bob", false).unwrap();
    let name = RepoName::parse("alice/secret").unwrap();
    repo::create(&store, &Actor::User("alice".into()), &name)
        .await
        .unwrap();
    let err = acl::require(&store, &Actor::User("bob".into()), &name, Role::Read).unwrap_err();
    assert!(err.to_string().contains("no access"));
}

#[tokio::test]
async fn request_refs_round_trip() {
    if !git_ok() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path());
    store.ensure_layout().unwrap();
    store.add_user("alice", true).unwrap();
    let name = RepoName::parse("alice/app").unwrap();
    let sha = seed_repo(
        &store,
        &name,
        "name: ci\non:\n  request:\njobs:\n  ok:\n    steps:\n      - run: true\n",
    )
    .await;
    let bare = store.repo_path(&name);
    git::git(&bare, &["branch", "feature", "HEAD"])
        .await
        .unwrap();
    let meta = request::create(
        &store,
        &Actor::User("alice".into()),
        &name,
        "feature",
        Some("master"),
        "Add feature",
        Some("body"),
    )
    .await
    .unwrap();
    assert_eq!(meta.id, 1);
    assert_eq!(meta.state, "open");
    let loaded = request::show(&store, &Actor::User("alice".into()), &name, 1)
        .await
        .unwrap();
    assert_eq!(loaded.title, "Add feature");
    let head = git::rev_parse(&bare, "refs/rabun/requests/1/head")
        .await
        .unwrap();
    assert_eq!(head, sha);
    request::review(
        &store,
        &Actor::User("alice".into()),
        &name,
        1,
        "approve",
        Some("lgtm"),
    )
    .await
    .unwrap();
    let merged = request::merge(&store, &Actor::User("alice".into()), &name, 1)
        .await
        .unwrap();
    assert_eq!(merged.state, "merged");
}

#[tokio::test]
async fn workflow_parse_and_protected_hook() {
    let yaml = r#"
name: ci
on:
  push:
    branches: [master]
  tag:
  request:
jobs:
  test:
    steps:
      - run: true
"#;
    let wf = workflow::parse(yaml).unwrap();
    assert!(wf.matches(workflow::Event::Push, Some("master")));
    assert!(!wf.matches(workflow::Event::Push, Some("dev")));

    std::env::remove_var("RABUN_GIT_USER");
    std::env::remove_var("RABUN_GIT_REPO");
    std::env::remove_var("RABUN_GIT_ROOT");
    rabun_git::hook::update("refs/heads/master", "0", "abc").unwrap();
}
