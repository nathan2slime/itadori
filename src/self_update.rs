use anyhow::{bail, Context, Result};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::process::Command;

const DEFAULT_UPDATE_REPO: Option<&str> = option_env!("ITADORI_UPDATE_REPO");

#[derive(Debug)]
pub struct SelfUpdateOptions {
    pub repo: Option<String>,
    pub asset: Option<String>,
    pub token: Option<String>,
    pub force: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    url: String,
}

pub async fn run(options: SelfUpdateOptions) -> Result<()> {
    let repo = resolve_repo(options.repo)?;
    let asset_name = options.asset.unwrap_or_else(default_asset_name);
    let client = reqwest::Client::builder().build()?;

    println!("checking latest release for {repo}");
    let release = fetch_latest_release(&client, &repo, options.token.as_deref()).await?;
    let latest_version = normalize_version(&release.tag_name);
    let current_version = normalize_version(env!("CARGO_PKG_VERSION"));

    if latest_version == current_version && !options.force {
        println!(
            "itadori is already up to date ({})",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .with_context(|| {
            let available = release
                .assets
                .iter()
                .map(|asset| asset.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "release {} does not contain asset {}; available assets: {}",
                release.tag_name, asset_name, available
            )
        })?;

    println!("installing {} from {}", release.tag_name, release.html_url);
    install_asset(&client, asset, options.token.as_deref()).await?;
    println!("updated itadori to {}", release.tag_name);
    Ok(())
}

fn resolve_repo(repo: Option<String>) -> Result<String> {
    if let Some(repo) = repo {
        validate_repo(&repo)?;
        return Ok(repo);
    }

    if let Some(repo) = DEFAULT_UPDATE_REPO {
        validate_repo(repo)?;
        return Ok(repo.to_string());
    }

    bail!(
        "missing update repository; pass --repo owner/repo or build with ITADORI_UPDATE_REPO=owner/repo"
    );
}

fn validate_repo(repo: &str) -> Result<()> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();

    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        bail!("repository must use owner/repo format: {repo}");
    }

    Ok(())
}

async fn fetch_latest_release(
    client: &reqwest::Client,
    repo: &str,
    token: Option<&str>,
) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let request = github_request(client.get(url), token);
    let response = request
        .send()
        .await
        .context("failed to query GitHub releases")?;

    if !response.status().is_success() {
        bail!(
            "GitHub release lookup failed with status {}",
            response.status()
        );
    }

    response
        .json::<GitHubRelease>()
        .await
        .context("failed to parse GitHub release response")
}

async fn install_asset(
    client: &reqwest::Client,
    asset: &GitHubAsset,
    token: Option<&str>,
) -> Result<()> {
    let current_exe = std::env::current_exe().context("failed to locate current executable")?;
    let exe_dir = current_exe
        .parent()
        .context("current executable has no parent directory")?;
    let work_dir = exe_dir.join(format!(".itadori-update-{}", std::process::id()));

    if work_dir.exists() {
        fs::remove_dir_all(&work_dir)
            .with_context(|| format!("failed to clean {}", work_dir.display()))?;
    }
    fs::create_dir_all(&work_dir)
        .with_context(|| format!("failed to create {}", work_dir.display()))?;

    let result = install_asset_inner(client, asset, token, &current_exe, &work_dir).await;
    let cleanup = fs::remove_dir_all(&work_dir);

    match (result, cleanup) {
        (Ok(()), _) => Ok(()),
        (Err(err), Ok(())) => Err(err),
        (Err(err), Err(cleanup_err)) => {
            Err(err).with_context(|| format!("also failed to clean {}", cleanup_err))
        }
    }
}

async fn install_asset_inner(
    client: &reqwest::Client,
    asset: &GitHubAsset,
    token: Option<&str>,
    current_exe: &Path,
    work_dir: &Path,
) -> Result<()> {
    let archive_path = work_dir.join(&asset.name);
    download_asset(client, asset, token, &archive_path).await?;

    let candidate = if asset.name.ends_with(".tar.gz") {
        extract_tar_gz(&archive_path, work_dir)?;
        let binary_name = asset.name.trim_end_matches(".tar.gz");
        work_dir.join(binary_name)
    } else {
        archive_path.clone()
    };

    if !candidate.is_file() {
        bail!(
            "downloaded asset did not contain executable {}",
            candidate.display()
        );
    }

    replace_current_exe(&candidate, current_exe)
}

async fn download_asset(
    client: &reqwest::Client,
    asset: &GitHubAsset,
    token: Option<&str>,
    destination: &Path,
) -> Result<()> {
    let response = github_request(client.get(&asset.url), token)
        .header(ACCEPT, "application/octet-stream")
        .send()
        .await
        .with_context(|| format!("failed to download {}", asset.name))?;

    if !response.status().is_success() {
        bail!(
            "failed to download {} with status {}",
            asset.name,
            response.status()
        );
    }

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read {}", asset.name))?;
    fs::write(destination, bytes)
        .with_context(|| format!("failed to write {}", destination.display()))?;
    Ok(())
}

fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()
        .context("failed to run tar")?;

    if !status.success() {
        bail!("tar failed to extract {}", archive.display());
    }

    Ok(())
}

fn replace_current_exe(candidate: &Path, current_exe: &Path) -> Result<()> {
    let replacement = current_exe.with_extension("itadori-new");
    let backup = current_exe.with_extension("itadori-old");

    if replacement.exists() {
        fs::remove_file(&replacement)
            .with_context(|| format!("failed to remove {}", replacement.display()))?;
    }
    if backup.exists() {
        fs::remove_file(&backup)
            .with_context(|| format!("failed to remove {}", backup.display()))?;
    }

    fs::copy(candidate, &replacement).with_context(|| {
        format!(
            "failed to stage replacement binary at {}",
            replacement.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&replacement, permissions)
            .with_context(|| format!("failed to mark {} executable", replacement.display()))?;
    }

    fs::rename(current_exe, &backup).with_context(|| {
        format!(
            "failed to move current executable {} to {}",
            current_exe.display(),
            backup.display()
        )
    })?;

    if let Err(err) = fs::rename(&replacement, current_exe) {
        let rollback = fs::rename(&backup, current_exe);
        if let Err(rollback_err) = rollback {
            return Err(err).with_context(|| {
                format!(
                    "failed to install replacement and rollback failed: {}",
                    rollback_err
                )
            });
        }
        return Err(err).context("failed to install replacement binary");
    }

    let _ = fs::remove_file(&backup);
    Ok(())
}

fn github_request(
    request: reqwest::RequestBuilder,
    token: Option<&str>,
) -> reqwest::RequestBuilder {
    let request = request
        .header(USER_AGENT, concat!("itadori/", env!("CARGO_PKG_VERSION")))
        .header(ACCEPT, "application/vnd.github+json");

    match token {
        Some(token) if !token.trim().is_empty() => {
            request.header(AUTHORIZATION, format!("Bearer {}", token.trim()))
        }
        _ => request,
    }
}

fn default_asset_name() -> String {
    format!(
        "itadori-{}-{}.tar.gz",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn normalize_version(version: &str) -> &str {
    version.trim().trim_start_matches('v')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_repo_format() {
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("owner").is_err());
        assert!(validate_repo("owner/repo/extra").is_err());
        assert!(validate_repo("/repo").is_err());
        assert!(validate_repo("owner/").is_err());
    }

    #[test]
    fn normalizes_release_versions() {
        assert_eq!(normalize_version("v1.2.3"), "1.2.3");
        assert_eq!(normalize_version("  v1.2.3  "), "1.2.3");
        assert_eq!(normalize_version("1.2.3"), "1.2.3");
    }

    #[test]
    fn default_asset_name_matches_release_workflow() {
        assert_eq!(
            default_asset_name(),
            format!(
                "itadori-{}-{}.tar.gz",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        );
    }

    #[test]
    fn replaces_current_exe_with_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("itadori");
        let candidate = dir.path().join("itadori-new-download");

        fs::write(&current, "old").unwrap();
        fs::write(&candidate, "new").unwrap();

        replace_current_exe(&candidate, &current).unwrap();

        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
        assert!(!current.with_extension("itadori-old").exists());
    }
}
