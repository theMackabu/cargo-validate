use clap::{CommandFactory, Parser};
use colored::*;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::ops::Deref;
use std::process::{exit, Command};
use std::sync::LazyLock;
use toml::Value as TomlValue;

#[derive(Parser)]
#[clap(name = "cargo-validate", bin_name = "cargo")]
#[clap(version, about = "Cargo publish with confirmation")]
enum Cli {
    Validate {
        /// Arguments to pass to the original cargo publish command
        #[clap(last = true)]
        args: Vec<String>,
    },
}

struct Package {
    name: ColoredString,
    version: ColoredString,
    edition: ColoredString,
    license: Option<ColoredString>,
    description: Option<ColoredString>,
    repository: Option<ColoredString>,
    name_exists: bool,
    version_exists: bool,
}

struct ColoredLazy(LazyLock<ColoredString>);

impl Deref for ColoredLazy {
    type Target = ColoredString;

    fn deref(&self) -> &Self::Target { &*self.0 }
}

impl fmt::Display for ColoredLazy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Display::fmt(&**self, f) }
}

static INVALID: ColoredLazy = ColoredLazy(LazyLock::new(|| "✖".red()));
static VALID: ColoredLazy = ColoredLazy(LazyLock::new(|| "✔".green()));

fn get_or_prompt_username() -> io::Result<String> {
    let home_dir = home::home_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory not found"))?;
    let username_file = home_dir.join(".cargo").join("username");

    if username_file.exists() {
        return fs::read_to_string(username_file);
    }

    print!("Please enter your crates.io username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();

    fs::create_dir_all(username_file.parent().unwrap())?;
    fs::write(&username_file, &username)?;

    Ok(username)
}

fn check_crate_exists(name: &str, version: &str) -> io::Result<(bool, bool, Vec<String>)> {
    let client = Client::new();
    let crate_url = format!("https://crates.io/api/v1/crates/{}", name);
    let owners_url = format!("https://crates.io/api/v1/crates/{}/owner_user", name);

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("cargo-validate"));

    // Check if crate exists and get version information
    let crate_response = client
        .get(&crate_url)
        .headers(headers.clone())
        .send()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Failed to send request to crates.io API: {}", e)))?;

    match crate_response.status().as_u16() {
        200 => {
            let body: Value = crate_response
                .json()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse API response: {}", e)))?;

            let versions = body["versions"]
                .as_array()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid API response: 'versions' field is missing or not an array"))?;

            let version_exists = versions.iter().any(|v| v["num"].as_str() == Some(version));

            let owners_response = client
                .get(&owners_url)
                .headers(headers)
                .send()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Failed to send request to crates.io API: {}", e)))?;

            if !owners_response.status().is_success() {
                return Err(io::Error::new(io::ErrorKind::Other, format!("Failed to get crate owners. Status: {}", owners_response.status())));
            }

            let owners_body: Value = owners_response
                .json()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse API response: {}", e)))?;

            let owners = owners_body["users"]
                .as_array()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid API response: 'users' field is missing or not an array"))?
                .iter()
                .filter_map(|user| user["login"].as_str().map(String::from))
                .collect();

            Ok((true, version_exists, owners))
        }
        404 => Ok((false, false, Vec::new())),
        403 => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Access forbidden. This could be due to IP-based rate limiting or other restrictions by crates.io.",
        )),
        429 => Err(io::Error::new(io::ErrorKind::Other, "Rate limit exceeded for crates.io API. Please try again later.")),
        status => Err(io::Error::new(io::ErrorKind::Other, format!("Unexpected response from crates.io API. Status code: {}", status))),
    }
}

fn get_package_info() -> io::Result<Package> {
    let cargo_toml = std::fs::read_to_string("Cargo.toml")?;
    let parsed_toml: TomlValue = toml::from_str(&cargo_toml).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let package = parsed_toml["package"]
        .as_table()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing [package] section"))?;

    let name = package.get("name").and_then(|v| v.as_str());
    let version = package.get("version").and_then(|v| v.as_str());
    let edition = package.get("edition").and_then(|v| v.as_str());

    let (name, version, edition) = match (name, version, edition) {
        (Some(n), Some(v), Some(e)) => (n, v, e),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Publish cancelled: name, version, or edition do not exist in Cargo.toml")),
    };

    let current_username = get_or_prompt_username()?;
    let (crate_exists, version_exists, owners) = check_crate_exists(name, version)?;

    let name_exists = crate_exists && !owners.contains(&current_username);
    let name = if name_exists { format!("{name} {INVALID}") } else { format!("{name} {VALID}") }.into();
    let version = if version_exists { format!("{version} {INVALID}") } else { format!("{version} {VALID}") }.into();

    let edition = if edition != "2021" {
        format!("{edition} {}", "(did you mean 2021?)".bright_yellow())
    } else {
        format!("{edition} {VALID}")
    }
    .into();

    let license = package.get("license").and_then(|v| v.as_str()).map(|s| s.normal());
    let description = package.get("description").and_then(|v| v.as_str()).map(|s| s.normal());
    let repository = package.get("repository").and_then(|v| v.as_str()).map(|s| s.normal());

    Ok(Package {
        name,
        version,
        edition,
        license,
        description,
        repository,
        name_exists,
        version_exists,
    })
}

fn get_git_status() -> io::Result<String> {
    let output = Command::new("git").args(&["status", "--porcelain"]).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run(args: Vec<String>) -> io::Result<()> {
    let pkg = get_package_info()?;

    println!("{}", "Package Information:".magenta().bold());
    println!(" {}: {}", "Name".bright_magenta(), pkg.name);
    println!(" {}: {}", "Version".bright_magenta(), pkg.version);
    println!(" {}: {}", "Edition".bright_magenta(), pkg.edition);

    let mut missing_fields = Vec::new();

    if let Some(license) = pkg.license {
        println!(" {}: {}", "License".bright_magenta(), license);
    } else {
        missing_fields.push("license");
    }

    println!("\n{}", " Metadata:".magenta().bold());

    if let Some(repository) = pkg.repository {
        println!("  {}: {}", "Repository".bright_magenta(), repository);
    } else {
        missing_fields.push("repository");
    }

    if let Some(description) = pkg.description {
        println!("  {}: {}", "Description".bright_magenta(), description);
    } else {
        missing_fields.push("description");
    }

    if !missing_fields.is_empty() {
        println!("\n{} {}", "Package is missing:".bright_yellow(), missing_fields.join(", ").bright_yellow());
    }

    let git_status = get_git_status()?;

    if git_status.is_empty() {
        println!("\n{}", "No uncommitted git changes".bright_green().bold());
    } else {
        println!("\n{}", "Uncommitted git changes:".red().bold());
        for line in git_status.lines() {
            let (status, file) = line.split_at(2);
            println!(" {} {}", status.trim().bright_red(), file.trim());
        }
    }

    let mut full_command = Vec::from(["publish".to_string()]);
    full_command.extend(args);

    if pkg.name_exists {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "\nPublish cancelled: name already exists"));
    } else if pkg.version_exists {
        print!("\n{} {}{} ", "Are you sure you want to publish an existing version?".bright_blue().bold(), "(y/n)".bright_cyan(), ":");
    } else if !git_status.is_empty() {
        print!("\n{} {}{} ", "Are you sure you want to publish with dirty directory?".bright_blue().bold(), "(y/n)".bright_cyan(), ":");
        full_command.push("--allow-dirty".to_string());
    } else {
        print!("\n{} {}{} ", "Are you sure you want to publish?".bright_blue().bold(), "(y/n)".bright_cyan(), ":");
    }

    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().eq_ignore_ascii_case("y") {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "\nPublish cancelled."));
    }

    println!("{}", "\nProceeding with cargo publish...".green().bold());

    if !Command::new("cargo").args(&full_command).status()?.success() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "\nCargo publish failed"));
    }

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 2 && args[1] == "validate" {
        let publish_args = args[2..].to_vec();
        if let Err(err) = run(publish_args) {
            eprintln!("{}", err.to_string().red().bold());
            exit(1);
        }
    } else {
        let mut cmd = Cli::command();
        let _ = cmd.print_help();
    }
}
