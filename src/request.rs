//! Merge requests stored as `refs/rabun/requests/<id>/{head,base,meta}`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::acl::{self, Actor, Role};
use crate::git;
use crate::names::RepoName;
use crate::now_iso;
use crate::store::Store;
use crate::workflow;

/// YAML blob at `refs/rabun/requests/<id>/meta`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMeta {
    /// Sequential id starting at 1.
    pub id: u64,
    /// Short title.
    pub title: String,
    /// Author login (or `operator`).
    pub author: String,
    /// Optional body.
    #[serde(default)]
    pub body: String,
    /// `open`, `merged`, or `rejected`.
    pub state: String,
    /// Target branch name (no `refs/heads/`).
    pub base_branch: String,
    /// Source branch name when known.
    #[serde(default)]
    pub head_branch: String,
    /// RFC 3339.
    pub created_at: String,
    /// Reviews in order.
    #[serde(default)]
    pub reviews: Vec<Review>,
}

/// One review on a request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    /// Reviewer login.
    pub author: String,
    /// `approve`, `reject`, or `comment`.
    pub verdict: String,
    /// Free text.
    #[serde(default)]
    pub comment: String,
    /// RFC 3339.
    pub at: String,
}

/// Open a request from `--head` (branch or SHA).
pub async fn create(
    store: &Store,
    actor: &Actor,
    repo: &RepoName,
    head: &str,
    base: Option<&str>,
    title: &str,
    body: Option<&str>,
) -> Result<RequestMeta> {
    acl::require(store, actor, repo, Role::Write)?;
    let path = store.repo_path(repo);
    if !path.exists() {
        bail!("repository {repo} not found");
    }
    let head_sha = git::rev_parse(&path, head).await?;
    let base_branch = match base {
        Some(b) => b.trim_start_matches("refs/heads/").to_string(),
        None => git::default_branch(&path).await?,
    };
    let base_sha = git::rev_parse(&path, &format!("refs/heads/{base_branch}")).await?;
    let head_branch = if head.starts_with("refs/") || looks_like_sha(head) {
        String::new()
    } else {
        head.trim_start_matches("refs/heads/").to_string()
    };
    let id = next_id(&path).await?;
    let meta = RequestMeta {
        id,
        title: title.to_string(),
        author: actor.author().to_string(),
        body: body.unwrap_or("").to_string(),
        state: "open".into(),
        base_branch,
        head_branch,
        created_at: now_iso(),
        reviews: Vec::new(),
    };
    write_request(&path, &meta, &head_sha, &base_sha).await?;
    workflow::trigger(store, repo, workflow::Event::Request, &head_sha, id, None).await?;
    Ok(meta)
}

/// Promote `refs/rabun/requests/new/<branch>` after receive-pack.
pub async fn promote_new_refs(
    store: &Store,
    actor: &Actor,
    repo: &RepoName,
) -> Result<Vec<RequestMeta>> {
    let path = store.repo_path(repo);
    let refs = git::list_refs(&path).await?;
    let mut created = Vec::new();
    for (refname, sha) in refs {
        let Some(branch) = crate::names::request_new_ref(&refname) else {
            continue;
        };
        let base_branch = git::default_branch(&path).await?;
        let base_sha = git::rev_parse(&path, &format!("refs/heads/{base_branch}")).await?;
        let id = next_id(&path).await?;
        let meta = RequestMeta {
            id,
            title: format!("Request from {branch}"),
            author: actor.author().to_string(),
            body: String::new(),
            state: "open".into(),
            base_branch,
            head_branch: branch.to_string(),
            created_at: now_iso(),
            reviews: Vec::new(),
        };
        write_request(&path, &meta, &sha, &base_sha).await?;
        git::git(&path, &["update-ref", "-d", &refname]).await?;
        workflow::trigger(store, repo, workflow::Event::Request, &sha, id, None).await?;
        created.push(meta);
    }
    Ok(created)
}

/// All request metas (any state).
pub async fn list(store: &Store, actor: &Actor, repo: &RepoName) -> Result<Vec<RequestMeta>> {
    acl::require(store, actor, repo, Role::Read)?;
    let path = store.repo_path(repo);
    let refs = git::list_refs(&path).await?;
    let mut ids: Vec<u64> = refs.keys().filter_map(|r| parse_meta_ref(r)).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        if let Ok(meta) = load_meta(&path, id).await {
            out.push(meta);
        }
    }
    Ok(out)
}

/// Load one request.
pub async fn show(store: &Store, actor: &Actor, repo: &RepoName, id: u64) -> Result<RequestMeta> {
    acl::require(store, actor, repo, Role::Read)?;
    load_meta(&store.repo_path(repo), id).await
}

/// Append a review. `verdict` is approve, reject, or comment.
pub async fn review(
    store: &Store,
    actor: &Actor,
    repo: &RepoName,
    id: u64,
    verdict: &str,
    comment: Option<&str>,
) -> Result<RequestMeta> {
    acl::require(store, actor, repo, Role::Write)?;
    if !matches!(verdict, "approve" | "reject" | "comment") {
        bail!("verdict must be approve, reject, or comment");
    }
    if verdict == "comment" && comment.map(str::trim).unwrap_or("").is_empty() {
        bail!("--comment is required unless --approve or --reject");
    }
    let path = store.repo_path(repo);
    let mut meta = load_meta(&path, id).await?;
    if meta.state != "open" {
        bail!("request {id} is {}", meta.state);
    }
    if verdict == "reject" {
        meta.state = "rejected".into();
    }
    meta.reviews.push(Review {
        author: actor.author().to_string(),
        verdict: verdict.to_string(),
        comment: comment.unwrap_or("").to_string(),
        at: now_iso(),
    });
    let head = git::rev_parse(&path, &head_ref(id)).await?;
    let base = git::rev_parse(&path, &base_ref(id)).await?;
    write_request(&path, &meta, &head, &base).await?;
    Ok(meta)
}

/// Fast-forward `base_branch` to the request head.
pub async fn merge(store: &Store, actor: &Actor, repo: &RepoName, id: u64) -> Result<RequestMeta> {
    acl::require(store, actor, repo, Role::Admin)?;
    let path = store.repo_path(repo);
    let mut meta = load_meta(&path, id).await?;
    if meta.state != "open" {
        bail!("request {id} is {}", meta.state);
    }
    let head = git::rev_parse(&path, &head_ref(id)).await?;
    let branch_ref = format!("refs/heads/{}", meta.base_branch);
    let tip = git::rev_parse(&path, &branch_ref).await?;
    if !git::is_ancestor(&path, &tip, &head).await? {
        bail!("request {id} is not a fast-forward of {}", meta.base_branch);
    }
    git::git(&path, &["update-ref", &branch_ref, &head]).await?;
    meta.state = "merged".into();
    let base = git::rev_parse(&path, &base_ref(id)).await?;
    write_request(&path, &meta, &head, &base).await?;
    workflow::trigger(
        store,
        repo,
        workflow::Event::Push,
        &head,
        id,
        Some(&meta.base_branch),
    )
    .await?;
    Ok(meta)
}

/// Format a request for CLI output.
pub fn format_meta(meta: &RequestMeta) -> String {
    let mut s = format!(
        "#{id} [{state}] {title}\n  author: {author}\n  {head} -> {base}\n  created: {created}\n",
        id = meta.id,
        state = meta.state,
        title = meta.title,
        author = meta.author,
        head = if meta.head_branch.is_empty() {
            "(sha)".to_string()
        } else {
            meta.head_branch.clone()
        },
        base = meta.base_branch,
        created = meta.created_at,
    );
    if !meta.body.is_empty() {
        s.push_str("  ");
        s.push_str(&meta.body.replace('\n', "\n  "));
        s.push('\n');
    }
    for review in &meta.reviews {
        s.push_str(&format!(
            "  review: {} {} {}\n",
            review.author, review.verdict, review.comment
        ));
    }
    s
}

async fn next_id(repo: &std::path::Path) -> Result<u64> {
    let refs = git::list_refs(repo).await?;
    let max = refs
        .keys()
        .filter_map(|r| {
            r.strip_prefix("refs/rabun/requests/")
                .and_then(|rest| rest.split('/').next())
                .and_then(|id| id.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    Ok(max + 1)
}

async fn write_request(
    repo: &std::path::Path,
    meta: &RequestMeta,
    head: &str,
    base: &str,
) -> Result<()> {
    let yaml = serde_yml::to_string(meta).context("serialize request meta")?;
    let blob = git::hash_blob(repo, yaml.as_bytes()).await?;
    git::git(repo, &["update-ref", &head_ref(meta.id), head]).await?;
    git::git(repo, &["update-ref", &base_ref(meta.id), base]).await?;
    git::git(repo, &["update-ref", &meta_ref(meta.id), &blob]).await?;
    Ok(())
}

async fn load_meta(repo: &std::path::Path, id: u64) -> Result<RequestMeta> {
    let yaml = git::cat_file(repo, &meta_ref(id))
        .await
        .with_context(|| format!("request {id} not found"))?;
    serde_yml::from_str(&yaml).context("parse request meta")
}

fn head_ref(id: u64) -> String {
    format!("refs/rabun/requests/{id}/head")
}

fn base_ref(id: u64) -> String {
    format!("refs/rabun/requests/{id}/base")
}

fn meta_ref(id: u64) -> String {
    format!("refs/rabun/requests/{id}/meta")
}

fn parse_meta_ref(refname: &str) -> Option<u64> {
    let rest = refname.strip_prefix("refs/rabun/requests/")?;
    let (id, kind) = rest.split_once('/')?;
    if kind == "meta" {
        id.parse().ok()
    } else {
        None
    }
}

fn looks_like_sha(s: &str) -> bool {
    s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit())
}
