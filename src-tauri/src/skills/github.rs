//! GitHub repositories as sources of skills.
//!
//! # One tree call, then only the chosen files
//!
//! The first version walked a folder through the contents API, one request per
//! directory. GitHub allows 60 requests an hour without an account, so a repo
//! like `heygen-com/hyperframes` — seven skills beside a 240 MB LFS test suite —
//! spent that budget discovering what it could have read in one call, and a
//! repository archive would have downloaded the LFS content too. The git trees
//! endpoint answers the whole file list in **one** request for a ref; the file
//! bytes then come from `raw.githubusercontent.com`, which is not billed against
//! the API quota. One resolve, only the skill's files.
//!
//! # Sources accepted
//!
//! `owner/repo`, `owner/repo/skills/name`, a GitHub repository URL, a `/tree/`
//! or `/blob/` URL, and a `.zip` URL (which stays with [`super::install`]).
//! A ref may contain slashes and GitHub's URL does not say where the ref ends
//! and the path begins, so every split is tried, shortest ref first.
//!
//! # Discovery
//!
//! A skill is a folder holding `SKILL.md`. A repository link finds them where
//! the ecosystem puts them — the repository root, or `skills/` and
//! `data/skills/` up to three levels deep — and deliberately not under
//! `.claude/skills` or `.agents/skills`, which are install targets rather than
//! the published set. A direct folder link searches only that folder. The
//! shallowest skill shadows anything nested below it, so a `SKILL.md` at the
//! repository root is the whole repository's skill rather than one of many.
//!
//! # Installation
//!
//! The files are named exactly as an archive would name them and handed to
//! [`super::install::install_files_with`], so both sources pass one set of
//! containment, frontmatter and slug checks. A skill is installed on its own:
//! one failing does not discard the ones that worked.

use futures_util::future::join_all;

use super::install::{install_files_with, MAX_ARCHIVE_BYTES, MAX_ENTRIES, MAX_FILE_BYTES};
use super::{frontmatter, is_script, Skill};

const API: &str = "https://api.github.com";
const RAW: &str = "https://raw.githubusercontent.com";

/// Skills one repository may offer in the picker.
const MAX_SKILLS: usize = 100;

/// How deep below `skills/` (and `data/skills/`) a skill may sit.
const MAX_LEVELS: usize = 3;

/// Files fetched at once. A raw download is small and network-bound; six is
/// enough to stop a thirty-file skill feeling sequential without opening a
/// connection per file.
const CONCURRENCY: usize = 6;

/// Where a link points.
pub enum Source {
    /// A repository, and the ref/path splits that could reach what it names.
    Github(Repository),
    /// A `.zip` URL, which [`super::install`] already knows how to unpack.
    Zip(String),
}

pub struct Repository {
    owner: String,
    repo: String,
    /// `(ref, optional folder filter)`, in the order to try.
    candidates: Vec<(String, Option<String>)>,
}

/// One skill the picker can offer.
#[derive(Clone, Debug, serde::Serialize)]
pub struct FoundSkill {
    /// The folder inside the repository. Empty for a skill at the root.
    pub id: String,
    /// The label drawn on the row.
    pub name: String,
    pub description: Option<String>,
    /// Supporting files, excluding `SKILL.md`.
    pub file_count: usize,
    pub has_scripts: bool,
}

/// What a repository link resolved to.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Discovery {
    pub owner: String,
    pub repo: String,
    pub reference: String,
    /// The tree was too large for GitHub to list completely.
    pub truncated: bool,
    pub skills: Vec<FoundSkill>,
}

/// What an install did, per skill.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InstallReport {
    pub installed: Vec<Skill>,
    pub failed: Vec<FailedSkill>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct FailedSkill {
    pub id: String,
    pub message: String,
}

/// One step, emitted to the dialog.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Progress {
    /// `reading`, `downloading`, `installing` or `done`.
    pub phase: &'static str,
    pub message: String,
    pub done: usize,
    pub total: usize,
}

impl Progress {
    fn new(phase: &'static str, message: impl Into<String>, done: usize, total: usize) -> Self {
        Self {
            phase,
            message: message.into(),
            done,
            total,
        }
    }
}

/// One file in a tree listing, read for the four fields the fetch needs.
#[derive(Clone, Debug, serde::Deserialize)]
struct Entry {
    path: String,
    #[serde(default)]
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: u64,
}

/// A skill folder and the files that belong to it.
#[derive(Clone, Debug)]
struct Plan {
    folder: String,
    skill_md: String,
    files: Vec<File>,
}

#[derive(Clone, Debug)]
struct File {
    path: String,
    size: u64,
}

/// Why a request could not be read.
enum Failure {
    /// The ref does not exist; another candidate may.
    NotFound,
    /// A real failure, reported as it is.
    Message(String),
}

/// A link, as a source of skills or as an archive.
pub fn parse(input: &str) -> Result<Source, String> {
    let input = input.trim();
    // The ecosystem's command is `npx skills add owner/repo`; copying the whole
    // line is a thing people do, so the command is not the part that is refused.
    let input = input
        .strip_prefix("npx skills add ")
        .or_else(|| input.strip_prefix("skills add "))
        .unwrap_or(input)
        .trim();
    if input.is_empty() {
        return Err("Paste a GitHub repository, owner/repo, or a .zip link.".to_string());
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        let url =
            reqwest::Url::parse(input).map_err(|_| "That is not a valid link.".to_string())?;
        // An archive URL is an archive before it is a GitHub page: the folder
        // link is the better door, but it is not the one this link opened.
        if input.to_lowercase().contains(".zip") {
            return Ok(Source::Zip(input.to_string()));
        }
        let host = url.host_str().unwrap_or_default().to_lowercase();
        if host != "github.com" && host != "www.github.com" {
            return Err("That link is neither a GitHub repository nor a .zip archive.".to_string());
        }
        let segments: Vec<&str> = url
            .path_segments()
            .map(|segments| segments.filter(|part| !part.is_empty()).collect())
            .unwrap_or_default();
        return repository(&segments, false).map(Source::Github);
    }
    // A shorthand is `owner/repo`, optionally with a path. A local path is not
    // a GitHub repository, and guessing one from it would be a surprise.
    if input.chars().any(char::is_whitespace)
        || input.starts_with('.')
        || input.starts_with('/')
        || input.contains('\\')
        || input.contains(':')
    {
        return Err("Paste a GitHub repository, owner/repo, or a .zip link.".to_string());
    }
    let segments: Vec<&str> = input.split('/').filter(|part| !part.is_empty()).collect();
    if segments.len() < 2 {
        return Err("A shorthand needs at least owner/repo.".to_string());
    }
    repository(&segments, true).map(Source::Github)
}

/// Turns path segments into a repository and the ways to reach what they name.
///
/// `allow_path` is true only for a shorthand: a full GitHub URL with a path
/// that is not `/tree/` or `/blob/` names a repository page (`/issues`,
/// `/releases`), not a folder inside it.
fn repository(segments: &[&str], allow_path: bool) -> Result<Repository, String> {
    if segments.len() < 2 {
        return Err("That GitHub link does not name a repository.".to_string());
    }
    let owner = segments[0];
    let repo = segments[1].trim_end_matches(".git");
    if !clean(owner) || !clean(repo) {
        return Err("That GitHub link does not name a repository.".to_string());
    }
    let rest = &segments[2..];
    let mut candidates: Vec<(String, Option<String>)> = Vec::new();

    if rest.is_empty() {
        candidates.push(("HEAD".to_string(), None));
    } else if rest[0] == "tree" || rest[0] == "blob" {
        let tail = &rest[1..];
        // A `blob` link names a file; its parent folder is the skill.
        let tail = if rest[0] == "blob" && tail.len() > 1 {
            &tail[..tail.len() - 1]
        } else {
            tail
        };
        for length in 1..=tail.len() {
            let path = tail[length..].join("/");
            if path.is_empty() {
                continue;
            }
            candidates.push((tail[..length].join("/"), Some(path)));
        }
        if candidates.is_empty() {
            candidates.push(("HEAD".to_string(), None));
        }
    } else if allow_path {
        candidates.push(("HEAD".to_string(), Some(rest.join("/"))));
    } else {
        // `/issues`, `/releases`, the repository root: the repository itself.
        candidates.push(("HEAD".to_string(), None));
    }

    for (reference, filter) in &candidates {
        if !reference.split('/').all(clean) {
            return Err(
                "That GitHub link has characters this installer will not follow.".to_string(),
            );
        }
        if let Some(filter) = filter {
            if !filter.split('/').all(clean) {
                return Err(
                    "That GitHub link has characters this installer will not follow.".to_string(),
                );
            }
        }
    }

    Ok(Repository {
        owner: owner.to_string(),
        repo: repo.to_string(),
        candidates,
    })
}

/// Resolves a repository link to the skills it publishes.
pub async fn discover(
    http: &reqwest::Client,
    repository: &Repository,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Discovery, String> {
    let mut missed = Vec::new();
    for (reference, filter) in &repository.candidates {
        match tree(http, repository, reference).await {
            Ok((entries, truncated)) => {
                let plans = plan(&entries, filter.as_deref());
                if plans.is_empty() {
                    missed.push(match filter {
                        Some(path) => format!("`{path}` on `{reference}`"),
                        None => format!("any skill on `{reference}`"),
                    });
                    continue;
                }
                let total = plans.len();
                progress(Progress::new(
                    "reading",
                    format!("Reading {total} skill{}", plural(total)),
                    0,
                    total,
                ));
                let mut skills = Vec::with_capacity(total);
                let mut read = 0;
                for chunk in plans.chunks(CONCURRENCY) {
                    let described = join_all(
                        chunk
                            .iter()
                            .map(|plan| describe(http, repository, reference, plan.clone())),
                    )
                    .await;
                    skills.extend(described);
                    read += chunk.len();
                    progress(Progress::new(
                        "reading",
                        format!("Read {read}/{total} skill{}", plural(total)),
                        read,
                        total,
                    ));
                }
                skills.sort_by(|left, right| {
                    left.name
                        .to_lowercase()
                        .cmp(&right.name.to_lowercase())
                        .then_with(|| left.id.cmp(&right.id))
                });
                return Ok(Discovery {
                    owner: repository.owner.clone(),
                    repo: repository.repo.clone(),
                    reference: reference.clone(),
                    truncated,
                    skills,
                });
            }
            Err(Failure::NotFound) => missed.push(format!("`{reference}`")),
            Err(Failure::Message(message)) => return Err(message),
        }
    }
    Err(format!(
        "GitHub has no skills at {}. Check the branch and the path in {}/{}.",
        missed.join(" or "),
        repository.owner,
        repository.repo
    ))
}

/// Downloads and installs the selected skills.
///
/// Each one is installed on its own, so a failed skill is reported beside the
/// ones that succeeded rather than aborting the batch.
pub async fn install_selected(
    http: &reqwest::Client,
    repository: &Repository,
    reference: &str,
    ids: &[String],
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<InstallReport, String> {
    let (entries, _) =
        tree(http, repository, reference)
            .await
            .map_err(|failure| match failure {
                Failure::NotFound => {
                    "That branch is no longer there. Discover the repository again.".to_string()
                }
                Failure::Message(message) => message,
            })?;
    let filter = repository
        .candidates
        .iter()
        .find(|(candidate, _)| candidate == reference)
        .and_then(|(_, filter)| filter.clone());
    let plans = plan(&entries, filter.as_deref());
    let total: usize = plans
        .iter()
        .filter(|plan| ids.contains(&plan.folder))
        .map(|plan| plan.files.len() + 1)
        .sum();

    let mut report = InstallReport {
        installed: Vec::new(),
        failed: Vec::new(),
    };
    let mut done = 0;
    for id in ids {
        let Some(plan) = plans.iter().find(|plan| &plan.folder == id) else {
            report.failed.push(FailedSkill {
                id: id.clone(),
                message: "That folder is no longer in the repository.".to_string(),
            });
            continue;
        };
        let label = label_of(repository, &plan.folder);
        progress(Progress::new(
            "downloading",
            format!("Downloading {label}"),
            done,
            total,
        ));
        match download_skill(
            http, repository, reference, plan, &mut done, total, &label, progress,
        )
        .await
        {
            Ok(files) => {
                progress(Progress::new(
                    "installing",
                    format!("Installing {label}"),
                    done,
                    total,
                ));
                match install_files_with(&files, &super::root()) {
                    Ok(skill) => report.installed.push(skill),
                    Err(message) => report.failed.push(FailedSkill {
                        id: id.clone(),
                        message,
                    }),
                }
            }
            Err(message) => report.failed.push(FailedSkill {
                id: id.clone(),
                message,
            }),
        }
    }
    progress(Progress::new(
        "done",
        format!(
            "{} installed{}",
            report.installed.len(),
            match report.failed.len() {
                0 => String::new(),
                failed => format!(", {failed} failed"),
            }
        ),
        done,
        total,
    ));
    Ok(report)
}

/// One skill's files, named as an archive would name them.
async fn download_skill(
    http: &reqwest::Client,
    repository: &Repository,
    reference: &str,
    plan: &Plan,
    done: &mut usize,
    total: usize,
    label: &str,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let declared: u64 = plan.files.iter().map(|file| file.size).sum();
    if declared > MAX_ARCHIVE_BYTES as u64 {
        return Err(format!(
            "{label} is {declared} bytes; a skill is limited to {MAX_ARCHIVE_BYTES}. \
             Paste the skill's own folder link or its .zip."
        ));
    }
    if plan.files.len() + 1 > MAX_ENTRIES {
        return Err(format!(
            "{label} holds {} files; the limit is {MAX_ENTRIES}.",
            plan.files.len() + 1
        ));
    }
    let prefix = if plan.folder.is_empty() {
        repository.repo.clone()
    } else {
        plan.folder
            .rsplit('/')
            .next()
            .unwrap_or(&repository.repo)
            .to_string()
    };

    let skill_md = download_path(http, repository, reference, &plan.skill_md)
        .await
        .map_err(|failure| failure.message())?;
    let mut files = vec![(format!("{prefix}/SKILL.md"), skill_md)];
    *done += 1;
    progress(Progress::new(
        "downloading",
        format!("Downloading {label}"),
        *done,
        total,
    ));

    for chunk in plan.files.chunks(CONCURRENCY) {
        let downloaded = join_all(
            chunk
                .iter()
                .map(|file| download_path(http, repository, reference, &file.path)),
        )
        .await;
        let mut batch = Vec::with_capacity(chunk.len());
        for (file, result) in chunk.iter().zip(downloaded) {
            let bytes = result.map_err(|failure| failure.message())?;
            let relative = file
                .path
                .strip_prefix(&format!("{}/", plan.folder))
                .unwrap_or(&file.path);
            batch.push((format!("{prefix}/{relative}"), bytes));
        }
        files.extend(batch);
        *done += chunk.len();
        progress(Progress::new(
            "downloading",
            format!("Downloading {label}"),
            *done,
            total,
        ));
    }
    Ok(files)
}

/// Reads one `SKILL.md` for the picker's label and trigger.
async fn describe(
    http: &reqwest::Client,
    repository: &Repository,
    reference: &str,
    plan: Plan,
) -> FoundSkill {
    let header = match download_path(http, repository, reference, &plan.skill_md).await {
        Ok(bytes) => std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| frontmatter::parse(text).ok())
            .map(|(header, _)| header)
            .unwrap_or_default(),
        Err(_) => frontmatter::Frontmatter::default(),
    };
    let folder = plan
        .folder
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&repository.repo);
    let name = if header.name.trim().is_empty() {
        folder.to_string()
    } else {
        header.name
    };
    let marker = format!("{}/", plan.folder);
    FoundSkill {
        id: plan.folder,
        name: frontmatter::humanize(&name),
        description: (!header.description.trim().is_empty())
            .then(|| header.description.trim().to_string()),
        file_count: plan.files.len(),
        has_scripts: plan.files.iter().any(|file| {
            let relative = file.path.strip_prefix(&marker).unwrap_or(&file.path);
            is_script(relative)
        }),
    }
}

fn label_of(repository: &Repository, folder: &str) -> String {
    let name = folder
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&repository.repo);
    frontmatter::humanize(name)
}

/// Finds the skills in a tree listing.
///
/// Files and folders are sorted by depth first, so the shallowest `SKILL.md`
/// wins and everything nested below it is skipped: a skill is the whole folder,
/// not the folders inside it.
fn plan(entries: &[Entry], filter: Option<&str>) -> Vec<Plan> {
    let mut found: Vec<(String, String)> = entries
        .iter()
        .filter(|entry| entry.kind == "blob" && entry.mode != "120000")
        .filter_map(|entry| {
            if entry.path == "SKILL.md" {
                Some((String::new(), entry.path.clone()))
            } else {
                entry
                    .path
                    .strip_suffix("/SKILL.md")
                    .map(|folder| (folder.to_string(), entry.path.clone()))
            }
        })
        .filter(|(folder, _)| match filter {
            Some(filter) => folder == filter || folder.starts_with(&format!("{filter}/")),
            None => standard(folder),
        })
        .collect();
    found.sort_by(|left, right| {
        depth(&left.0)
            .cmp(&depth(&right.0))
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut chosen: Vec<(String, String)> = Vec::new();
    for (folder, markdown) in found {
        let shadowed = chosen.iter().any(|(outer, _)| {
            outer.is_empty() || folder == *outer || folder.starts_with(&format!("{outer}/"))
        });
        if shadowed {
            continue;
        }
        chosen.push((folder, markdown));
        if chosen.len() >= MAX_SKILLS {
            break;
        }
    }

    chosen
        .into_iter()
        .map(|(folder, skill_md)| Plan {
            files: files_under(entries, &folder),
            folder,
            skill_md,
        })
        .collect()
}

/// Where the ecosystem publishes skills, and nowhere else.
///
/// `.claude/skills` and `.agents/skills` are deliberately absent: they are the
/// directories an installer writes *into*, so reading them would offer a
/// repository's own installed copy as if it were a published skill.
fn standard(folder: &str) -> bool {
    if folder.is_empty() {
        return true;
    }
    ["skills", "data/skills"].iter().any(|base| {
        folder
            .strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(|rest| !rest.is_empty() && rest.split('/').count() <= MAX_LEVELS)
            .unwrap_or(folder == *base)
    })
}

/// The supporting files of one skill folder.
///
/// A skill at the repository root has no folder to bound it, so it installs as
/// its `SKILL.md` alone. A skill with assets belongs in a folder of its own;
/// copying the repository's README and license into the skill would be worse
/// than leaving them behind.
fn files_under(entries: &[Entry], folder: &str) -> Vec<File> {
    if folder.is_empty() {
        return Vec::new();
    }
    let prefix = format!("{folder}/");
    entries
        .iter()
        .filter(|entry| entry.kind == "blob" && entry.mode != "120000")
        .filter(|entry| entry.path.starts_with(&prefix))
        .filter(|entry| entry.path != format!("{prefix}SKILL.md"))
        .map(|entry| File {
            path: entry.path.clone(),
            size: entry.size,
        })
        .collect()
}

fn depth(folder: &str) -> usize {
    if folder.is_empty() {
        0
    } else {
        folder.split('/').count()
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// A whole repository's file list for one ref.
async fn tree(
    http: &reqwest::Client,
    repository: &Repository,
    reference: &str,
) -> Result<(Vec<Entry>, bool), Failure> {
    let url = tree_url(repository, reference).map_err(Failure::Message)?;
    let response = http
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| Failure::Message(format!("GitHub did not answer: {error}")))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Failure::NotFound);
    }
    if !status.is_success() {
        return Err(Failure::Message(rate_message(response)));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| Failure::Message(format!("GitHub sent something unreadable: {error}")))?;
    let tree = value
        .get("tree")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Failure::Message("GitHub sent a tree without files".to_string()))?;
    let entries = tree
        .iter()
        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
        .collect();
    let truncated = value
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Ok((entries, truncated))
}

/// One file's bytes.
async fn download_path(
    http: &reqwest::Client,
    repository: &Repository,
    reference: &str,
    path: &str,
) -> Result<Vec<u8>, Failure> {
    let url = raw_url(repository, reference, path).map_err(Failure::Message)?;
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| Failure::Message(format!("Could not download `{path}`: {error}")))?;
    if !response.status().is_success() {
        return Err(Failure::Message(format!(
            "GitHub answered {} for `{path}`.",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| Failure::Message(format!("`{path}` did not arrive intact: {error}")))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(Failure::Message(format!(
            "`{path}` is larger than a skill file may be"
        )));
    }
    Ok(bytes.to_vec())
}

fn tree_url(repository: &Repository, reference: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(API)
        .map_err(|error| format!("Could not build the GitHub request: {error}"))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not build the GitHub request".to_string())?;
        segments.push("repos");
        segments.push(&repository.owner);
        segments.push(&repository.repo);
        segments.push("git");
        segments.push("trees");
        segments.push(reference);
    }
    url.query_pairs_mut().append_pair("recursive", "1");
    Ok(url)
}

fn raw_url(repository: &Repository, reference: &str, path: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(RAW)
        .map_err(|error| format!("Could not build the GitHub request: {error}"))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not build the GitHub request".to_string())?;
        segments.push(&repository.owner);
        segments.push(&repository.repo);
        for part in reference.split('/') {
            segments.push(part);
        }
        for part in path.split('/') {
            segments.push(part);
        }
    }
    Ok(url.to_string())
}

/// The rate-limit answer, with the moment it ends rather than a shrug.
fn rate_message(response: reqwest::Response) -> String {
    let status = response.status();
    let limited = status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
    if !limited {
        return format!("GitHub answered {status}.");
    }
    let reset = response
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string());
    match reset {
        Some(at) => format!(
            "GitHub's API quota is spent; it resets at {at}. Paste the skill's .zip link, or \
             try again then."
        ),
        None => "GitHub's API quota is spent. Paste the skill's .zip link, or try again later."
            .to_string(),
    }
}

impl Failure {
    fn message(self) -> String {
        match self {
            Failure::NotFound => "GitHub has no such file.".to_string(),
            Failure::Message(message) => message,
        }
    }
}

/// The HTTP client both GitHub doors use.
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .user_agent("projectz-skills/1")
        .build()
        .map_err(|error| format!("Could not start the download: {error}"))
}

/// Resolves a link the dialog pasted. A `.zip` is not a repository.
pub async fn discover_url(
    url: &str,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Discovery, String> {
    match parse(url)? {
        Source::Github(repository) => discover(&client()?, &repository, progress).await,
        Source::Zip(_) => Err(
            "That is a .zip link, which installs directly. Paste a GitHub repository or \
             owner/repo to choose skills."
                .to_string(),
        ),
    }
}

/// Installs the selected folders from a repository link.
pub async fn install_url_selected(
    url: &str,
    reference: &str,
    ids: &[String],
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<InstallReport, String> {
    match parse(url)? {
        Source::Github(repository) => {
            install_selected(&client()?, &repository, reference, ids, progress).await
        }
        Source::Zip(_) => Err(
            "That is a .zip link, which installs directly rather than by selection.".to_string(),
        ),
    }
}

/// A segment that can be addressed as one path component.
///
/// `%` is refused rather than re-encoded: a link copied from GitHub's address
/// bar is plain, and pushing an already-encoded segment would encode the `%`
/// again into a path that exists nowhere.
fn clean(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains('\\')
        && !part.contains('/')
        && !part.contains('%')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github(input: &str) -> Repository {
        match parse(input).expect("parse") {
            Source::Github(repository) => repository,
            Source::Zip(_) => panic!("expected a repository"),
        }
    }

    fn entry(path: &str, kind: &str) -> Entry {
        Entry {
            path: path.to_string(),
            mode: if kind == "blob" { "100644" } else { "040000" }.to_string(),
            kind: kind.to_string(),
            size: 0,
        }
    }

    fn folders(entries: &[Entry]) -> Vec<String> {
        plan(entries, None)
            .into_iter()
            .map(|plan| plan.folder)
            .collect()
    }

    #[test]
    fn a_shorthand_names_the_repository() {
        let repository = github("heygen-com/hyperframes");
        assert_eq!(repository.owner, "heygen-com");
        assert_eq!(repository.repo, "hyperframes");
        assert_eq!(repository.candidates, vec![("HEAD".to_string(), None)]);
    }

    /// Copying the command line rather than the shorthand is the same request.
    #[test]
    fn a_copied_command_is_understood() {
        let repository = github("npx skills add heygen-com/hyperframes");
        assert_eq!(repository.owner, "heygen-com");
        assert_eq!(repository.repo, "hyperframes");
        assert_eq!(repository.candidates, vec![("HEAD".to_string(), None)]);
    }

    #[test]
    fn a_shorthand_may_name_a_folder() {
        let repository = github("latent-spaces/brag/skills/brag-slim");
        assert_eq!(
            repository.candidates,
            vec![("HEAD".to_string(), Some("skills/brag-slim".to_string()))]
        );
    }

    /// A branch may contain a slash and the URL does not say where it ends.
    /// Every split is offered, shortest ref first.
    #[test]
    fn a_tree_link_offers_every_ref_split() {
        let repository = github("https://github.com/owner/repo/tree/feature/pdf/skills/pdf");
        assert_eq!(
            repository.candidates,
            vec![
                ("feature".to_string(), Some("pdf/skills/pdf".to_string())),
                ("feature/pdf".to_string(), Some("skills/pdf".to_string())),
                ("feature/pdf/skills".to_string(), Some("pdf".to_string())),
            ]
        );
    }

    #[test]
    fn a_blob_link_uses_the_folder_that_holds_the_file() {
        let repository = github("https://github.com/owner/repo/blob/main/skills/pdf/SKILL.md");
        assert_eq!(
            repository.candidates,
            vec![
                ("main".to_string(), Some("skills/pdf".to_string())),
                ("main/skills".to_string(), Some("pdf".to_string())),
            ]
        );
    }

    #[test]
    fn a_repository_link_and_its_pages_are_the_repository() {
        for link in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo/issues",
            "https://github.com/owner/repo/tree/main",
        ] {
            let repository = github(link);
            assert_eq!(repository.candidates, vec![("HEAD".to_string(), None)]);
        }
    }

    #[test]
    fn a_zip_link_stays_an_archive() {
        assert!(matches!(
            parse("https://example.test/skill.zip").expect("parse"),
            Source::Zip(_)
        ));
        assert!(matches!(
            parse("https://github.com/owner/repo/archive/refs/heads/main.zip").expect("parse"),
            Source::Zip(_)
        ));
    }

    #[test]
    fn what_is_not_a_link_is_refused() {
        assert!(parse("").is_err());
        assert!(parse("nonsense").is_err());
        assert!(parse("./local/skill").is_err());
        assert!(parse("C:/Users/me/skill").is_err());
        assert!(parse("https://example.test/skills/pdf").is_err());
        assert!(parse("https://github.com/owner").is_err());
    }

    /// Discovery looks where skills are published, not where an installer
    /// writes them, and the shallowest skill shadows everything below it.
    #[test]
    fn discovery_finds_standard_containers_and_respects_shadowing() {
        let entries = vec![
            entry("README.md", "blob"),
            entry("skills/brag/SKILL.md", "blob"),
            entry("skills/brag-slim/SKILL.md", "blob"),
            entry("skills/category/deep/SKILL.md", "blob"),
            entry("skills/a/b/c/SKILL.md", "blob"),
            entry("skills/a/b/c/d/too-deep/SKILL.md", "blob"),
            entry("data/skills/extra/SKILL.md", "blob"),
            entry(".claude/skills/brag/SKILL.md", "blob"),
            entry(".agents/skills/brag/SKILL.md", "blob"),
        ];
        assert_eq!(
            folders(&entries),
            vec![
                "skills/brag",
                "skills/brag-slim",
                "data/skills/extra",
                "skills/category/deep",
                "skills/a/b/c",
            ]
        );
    }

    #[test]
    fn a_root_skill_shadows_the_rest_of_the_repository() {
        let entries = vec![
            entry("SKILL.md", "blob"),
            entry("skills/brag/SKILL.md", "blob"),
        ];
        assert_eq!(folders(&entries), vec![""]);
    }

    #[test]
    fn a_folder_link_searches_only_that_folder() {
        let entries = vec![
            entry("skills/brag/SKILL.md", "blob"),
            entry("skills/brag-slim/SKILL.md", "blob"),
            entry("skills/brag-slim/references/notes.md", "blob"),
        ];
        let plans = plan(&entries, Some("skills/brag-slim"));
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].folder, "skills/brag-slim");
        assert_eq!(plans[0].files.len(), 1);
        assert!(plans[0].files[0].path.ends_with("notes.md"));
    }

    #[test]
    fn a_symlinked_skill_file_is_not_a_skill() {
        let mut link = entry(".agents/skills/brag", "blob");
        link.mode = "120000".to_string();
        let entries = vec![link, entry("skills/real/SKILL.md", "blob")];
        assert_eq!(folders(&entries), vec!["skills/real"]);
    }

    #[test]
    fn supporting_files_stay_inside_their_skill() {
        let entries = vec![
            entry("skills/a/SKILL.md", "blob"),
            entry("skills/a/scripts/run.py", "blob"),
            entry("skills/a-b/SKILL.md", "blob"),
            entry("skills/a-b/scripts/other.py", "blob"),
        ];
        let plans = plan(&entries, None);
        let a = plans
            .iter()
            .find(|plan| plan.folder == "skills/a")
            .expect("skill a");
        assert_eq!(a.files.len(), 1);
        assert_eq!(a.files[0].path, "skills/a/scripts/run.py");
    }
}
