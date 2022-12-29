use anyhow::Context as _;
use cmd_lib::run_cmd;

async fn list_org_repos(
    octocrab: &mut octocrab::Octocrab,
    name: &str,
) -> Result<octocrab::Page<octocrab::models::Repository>, octocrab::Error> {
    octocrab
        .orgs(name)
        .list_repos()
        .repo_type(octocrab::params::repos::Type::All)
        .sort(octocrab::params::repos::Sort::FullName)
        .direction(octocrab::params::Direction::Ascending)
        .per_page(100)
        .send()
        .await
}

#[derive(serde::Serialize)]
pub struct ListUserReposParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<octocrab::params::repos::Sort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    direction: Option<octocrab::params::Direction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_page: Option<u8>,
}

async fn list_user_repos(
    octocrab: &mut octocrab::Octocrab,
    name: &str,
) -> Result<octocrab::Page<octocrab::models::Repository>, octocrab::Error> {
    let url = format!("users/{}/repos", name);
    let params = ListUserReposParameters {
        sort: Some(octocrab::params::repos::Sort::FullName),
        direction: Some(octocrab::params::Direction::Ascending),
        per_page: Some(100),
    };
    octocrab.get(url, Some(&params)).await
}

async fn fetch_all_repo_pages(
    octocrab: &mut octocrab::Octocrab,
    mut current_page: octocrab::Page<octocrab::models::Repository>,
) -> anyhow::Result<Vec<octocrab::models::Repository>> {
    let mut repos = current_page.take_items();

    while let Some(mut new_page) = octocrab
        .get_page(&current_page.next)
        .await
        .context("failed to load next page")?
    {
        repos.extend(new_page.take_items());

        current_page = new_page;
    }

    Ok(repos)
}

/// Mirror git repositories
#[derive(argh::FromArgs)]
struct Args {
    /// user to mirror repos from
    #[argh(option, long = "user", short = 'u', arg_name = "USER")]
    users: Vec<String>,
    /// organisation to mirror repos from
    #[argh(option, long = "org", short = 'o', arg_name = "ORG")]
    orgs: Vec<String>,
    /// destination directory for the mirrors
    #[argh(option, short = 'd')]
    destination: std::path::PathBuf,
    /// fetch LFS objects
    #[argh(switch)]
    lfs: bool,
}

async fn process_repos(args: &Args, repos: &[octocrab::models::Repository]) -> anyhow::Result<()> {
    for repo in repos {
        let full_name = repo.full_name.as_ref().context("missing full_name")?;
        log::info!("process repo `{}`", full_name);

        let url = repo.clone_url.as_ref().context("missing clone_url")?;
        let destination = args.destination.join(full_name);

        if destination.exists() {
            run_cmd!(git -C "$destination" remote update --prune)?;
        } else {
            let parent_dir = destination
                .parent()
                .context("failed to get parent directory")?;
            std::fs::create_dir_all(parent_dir).context("failed to create parent directory")?;

            run_cmd!(git clone --mirror "$url" "$destination")?;
        }

        if args.lfs {
            log::info!("fetch LFS from `{}`", full_name);
            run_cmd!(git -C "$destination" lfs fetch --all)?;
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args: Args = argh::from_env();

    let mut builder = octocrab::Octocrab::builder();

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        builder = builder.personal_token(token);
    }

    let mut octocrab = builder.build().context("can't create octocrab instance")?;

    for user in &args.users {
        log::info!("fetch repository list of user `{}`", user);
        let page = list_user_repos(&mut octocrab, user).await?;
        let repos = fetch_all_repo_pages(&mut octocrab, page).await?;
        process_repos(&args, &repos).await?;
    }

    for org in &args.orgs {
        log::info!("fetch repository list of org `{}`", org);
        let page = list_org_repos(&mut octocrab, org).await?;
        let repos = fetch_all_repo_pages(&mut octocrab, page).await?;
        process_repos(&args, &repos).await?;
    }

    Ok(())
}
