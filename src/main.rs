use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(name = "mcp-registry", about = "Docker MCP Registry CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a Docker image for an MCP server
    Build {
        /// Server name
        server: String,
        /// List discovered tools
        #[arg(long)]
        tools: bool,
        /// Pull community (non-mcp/) images
        #[arg(long)]
        pull_community: bool,
    },
    /// Generate a catalog YAML for an MCP server
    Catalog {
        /// Server name
        server: String,
    },
    /// Create a new MCP server definition from a GitHub repo
    Create {
        /// GitHub repository URL
        url: String,
        /// Server name override
        #[arg(long)]
        name: Option<String>,
        /// Category
        #[arg(long)]
        category: String,
        /// Use existing Docker image instead of building
        #[arg(long)]
        image: Option<String>,
        /// Skip build step
        #[arg(long)]
        no_build: bool,
        /// Skip tool listing
        #[arg(long)]
        no_tools: bool,
        /// Extra args (-e KEY=VAL or command args)
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Validate an MCP server definition
    Validate {
        /// Server name
        #[arg(long)]
        name: String,
    },
    /// Interactive wizard to create a server definition
    Wizard,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Server {
    name: String,
    #[serde(default)]
    image: String,
    #[serde(default, rename = "type")]
    server_type: String,
    #[serde(default)]
    meta: Meta,
    #[serde(default)]
    about: About,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    run: Run,
    #[serde(default)]
    config: Config,
    #[serde(default)]
    remote: Remote,
    #[serde(default)]
    dynamic: Option<Dynamic>,
    #[serde(default)]
    tools: Vec<PociTool>,
    #[serde(default)]
    oauth: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Meta {
    #[serde(default)]
    category: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct About {
    #[serde(default)]
    icon: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Source {
    #[serde(default)]
    project: String,
    #[serde(default)]
    upstream: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    directory: String,
    #[serde(default)]
    build_target: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Run {
    #[serde(default)]
    command: Vec<String>,
    #[serde(default)]
    volumes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Config {
    #[serde(default)]
    description: String,
    #[serde(default)]
    secrets: Vec<Secret>,
    #[serde(default)]
    env: Vec<Env>,
    #[serde(default)]
    parameters: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Secret {
    name: String,
    env: String,
    #[serde(default)]
    example: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Env {
    name: String,
    #[serde(default)]
    example: String,
    #[serde(default)]
    value: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Remote {
    #[serde(default)]
    url: String,
    #[serde(default)]
    transport_type: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Dynamic {
    #[serde(default)]
    tools: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PociTool {
    #[serde(default)]
    container: PociContainer,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PociContainer {
    #[serde(default)]
    image: String,
}

fn read_server(name: &str) -> Result<Server, String> {
    let path = PathBuf::from("servers").join(name).join("server.yaml");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    serde_yaml::from_str(&content)
        .map_err(|e| format!("Invalid YAML in {}: {e}", path.display()))
}

fn docker(args: &[&str]) -> Result<(), String> {
    let status = Command::new("docker")
        .args(args)
        .status()
        .map_err(|e| format!("Failed to run docker: {e}"))?;
    if !status.success() {
        return Err(format!("docker {} failed", args.join(" ")));
    }
    Ok(())
}

fn docker_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run docker: {e}"))?;
    if !output.status.success() {
        return Err(format!("docker {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn github_api(client: &reqwest::Client, path: &str) -> Result<Value, String> {
    let url = if path.starts_with("https://") { path.to_string() } else { format!("https://api.github.com/{path}") };
    let mut req = client.get(&url).header("User-Agent", "mcp-registry");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("token {token}"));
    }
    let resp = req.send().await.map_err(|e| format!("GitHub API error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }
    resp.json().await.map_err(|e| format!("GitHub parse error: {e}"))
}

fn guess_name(url: &str) -> String {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    let mut name = parts.last().unwrap_or(&"unknown").to_lowercase();
    for prefix in &["mcp-server-", "mcp-", "server-"] {
        name = name.strip_prefix(prefix).unwrap_or(&name).to_string();
    }
    for suffix in &["-mcp-server", "-mcp", "-server"] {
        name = name.strip_suffix(suffix).unwrap_or(&name).to_string();
    }
    name
}

async fn cmd_build(server_name: &str, list_tools: bool, pull_community: bool) -> Result<(), String> {
    let server = read_server(server_name)?;

    if !server.remote.url.is_empty() {
        println!("Build skipped for remote server {server_name}");
        return Ok(());
    }
    if server.server_type == "poci" {
        println!("Build skipped for poci server {server_name}");
        return Ok(());
    }

    let is_mcp = server.image.starts_with("mcp/");
    if is_mcp {
        let mut git_url = format!("{}.git#", server.source.project);
        if !server.source.branch.is_empty() {
            git_url.push_str(&server.source.branch);
        }
        if !server.source.directory.is_empty() && server.source.directory != "." {
            git_url.push(':');
            git_url.push_str(&server.source.directory);
        }

        let mut args = vec!["buildx", "build", "-t", "check", "-t", &server.image, "--load"];
        args.push(&git_url);
        docker(&args)?;
        println!("Image built as {}", server.image);
    } else if pull_community {
        docker(&["pull", &server.image])?;
        println!("Image pulled as {}", server.image);
    } else {
        return Err(format!("Server image {} is not in mcp/ namespace. Use --pull-community to pull it.", server.image));
    }

    if list_tools {
        let tools_path = PathBuf::from("servers").join(server_name).join("tools.json");
        if tools_path.exists() {
            let content = std::fs::read_to_string(&tools_path).map_err(|e| format!("{e}"))?;
            let tools: Vec<Value> = serde_json::from_str(&content).map_err(|e| format!("{e}"))?;
            println!("\n{} tools found.", tools.len());
        }
    }

    Ok(())
}

fn cmd_catalog(server_name: &str) -> Result<(), String> {
    let server = read_server(server_name)?;

    let catalog_dir = PathBuf::from("catalogs").join(server_name);
    std::fs::create_dir_all(&catalog_dir).map_err(|e| format!("{e}"))?;

    let tile = serde_json::json!({
        "name": server.name,
        "title": server.about.title,
        "description": server.about.description,
        "icon": server.about.icon,
        "category": server.meta.category,
        "tags": server.meta.tags,
    });

    let catalog = serde_json::json!({
        "version": "v1",
        "name": "docker-mcp",
        "display_name": "Local Test Catalog",
        "registry": [{
            "name": server_name,
            "tile": tile,
        }],
    });

    let catalog_file = catalog_dir.join("catalog.yaml");
    let yaml = serde_yaml::to_string(&catalog).map_err(|e| format!("{e}"))?;
    std::fs::write(&catalog_file, yaml).map_err(|e| format!("{e}"))?;
    println!("Catalog written to {}", catalog_file.display());

    Ok(())
}

async fn cmd_create(
    url: &str, name: Option<&str>, category: &str, image: Option<&str>,
    build: bool, list_tools: bool, extra_args: &[String],
) -> Result<(), String> {
    let client = reqwest::Client::new();

    let guessed = guess_name(url);
    let server_name = name.unwrap_or(&guessed);

    let tag = image.unwrap_or(&format!("mcp/{server_name}")).to_string();
    let tag = if image.is_some() { image.unwrap().to_string() } else { format!("mcp/{server_name}") };

    let title = {
        let mut chars = guessed.chars();
        match chars.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().to_string() + chars.as_str(),
        }
    };

    // Parse extra args for -e KEY=VAL and command args
    let mut secrets = Vec::new();
    let mut env = Vec::new();
    let mut command = Vec::new();
    let mut i = 0;
    while i < extra_args.len() {
        if extra_args[i] == "-e" && i + 1 < extra_args.len() {
            i += 1;
            let kv = &extra_args[i];
            let parts: Vec<&str> = kv.splitn(2, '=').collect();
            if parts.len() == 2 {
                let key = parts[0];
                let val = parts[1];
                if key.ends_with("_TOKEN") || key.ends_with("_KEY") || key.ends_with("_PASSWORD") {
                    secrets.push(Secret {
                        name: format!("{server_name}.{}", key.to_lowercase()),
                        env: key.to_string(),
                        example: val.to_string(),
                    });
                } else {
                    env.push(Env { name: key.to_string(), example: val.to_string(), value: String::new() });
                }
            }
        } else {
            command.push(extra_args[i].clone());
        }
        i += 1;
    }

    if build && image.is_none() {
        let mut git_url = format!("{url}.git#");
        let mut args = vec!["buildx", "build", "-t", "check", "-t", &tag, "--load"];
        args.push(&git_url);
        docker(&args)?;
    }

    let server = Server {
        name: server_name.to_string(),
        image: tag,
        server_type: "server".to_string(),
        meta: Meta { category: category.to_string(), tags: vec![category.to_string()] },
        about: About { icon: String::new(), title: title.clone(), description: "TODO".to_string() },
        source: Source { project: url.to_string(), ..Default::default() },
        run: Run { command, ..Default::default() },
        config: Config { description: format!("Configure the connection to {title}"), secrets, env, parameters: None },
        ..Default::default()
    };

    let server_dir = PathBuf::from("servers").join(server_name);
    std::fs::create_dir_all(&server_dir).map_err(|e| format!("{e}"))?;

    let yaml = serde_yaml::to_string(&server).map_err(|e| format!("{e}"))?;
    let server_file = server_dir.join("server.yaml");
    std::fs::write(&server_file, &yaml).map_err(|e| format!("{e}"))?;

    println!("Server definition written to {}", server_file.display());
    println!("\nWhat to do next?");
    println!("  1. Review {} and fix any TODOs", server_file.display());
    println!("  2. mcp-registry build {server_name}");
    println!("  3. mcp-registry catalog {server_name}");
    println!("  4. docker mcp catalog import $PWD/catalogs/{server_name}/catalog.yaml");

    Ok(())
}

fn cmd_validate(name: &str) -> Result<(), String> {
    // Check name format
    let re = regex::Regex::new(r"^[a-z0-9-]+$").unwrap();
    if !re.is_match(name) {
        return Err("Name must be lowercase with only letters, numbers, and hyphens".to_string());
    }
    println!("Name is valid");

    // Check server.yaml exists
    let server = read_server(name)?;
    if server.name != name {
        return Err(format!("server.yaml name '{}' does not match '{name}'", server.name));
    }
    println!("Directory is valid");

    // Check secrets prefixed with server name
    for secret in &server.config.secrets {
        if !secret.name.starts_with(&format!("{name}.")) {
            return Err(format!("Secret '{}' must be prefixed with '{name}.'", secret.name));
        }
    }
    println!("Secrets are valid");

    // Check config env references
    for e in &server.config.env {
        if e.value.starts_with("{{") && !e.value.starts_with(&format!("{{{{{name}.")) {
            return Err(format!("Env '{}' uses unknown parameter reference: {}", e.name, e.value));
        }
    }
    println!("Config env is valid");

    // Check remote config
    if !server.remote.url.is_empty() {
        if server.remote.transport_type.is_empty() {
            return Err("Remote server must have transport_type".to_string());
        }
        let valid = ["stdio", "sse", "streamable-http"];
        if !valid.contains(&server.remote.transport_type.as_str()) {
            return Err(format!("Invalid transport_type: {}", server.remote.transport_type));
        }
        println!("Remote is valid");
    } else {
        println!("Remote validation skipped (not a remote server)");
    }

    // Check OAuth requires dynamic tools
    if !server.oauth.is_empty() {
        if server.dynamic.is_none() || !server.dynamic.as_ref().unwrap().tools {
            return Err("Server with OAuth must have dynamic.tools: true".to_string());
        }
    }
    println!("OAuth dynamic configuration is valid");

    // Check poci images
    if server.server_type == "poci" {
        for tool in &server.tools {
            if !tool.container.image.is_empty() {
                docker(&["pull", &tool.container.image])?;
            }
        }
        println!("Poci images are valid");
    }

    println!("\nAll validations passed!");
    Ok(())
}

fn cmd_wizard() -> Result<(), String> {
    println!("MCP Server Registry Wizard");
    println!("==========================\n");
    println!("This wizard helps you create an MCP server definition interactively.");
    println!("For non-interactive use, use the `create` command instead.\n");

    // Read from stdin for basic wizard flow
    let stdin = std::io::stdin();

    let read_input = |prompt: &str| -> String {
        eprint!("{prompt}: ");
        let mut input = String::new();
        stdin.lock().read_line(&mut input).unwrap_or(0);
        input.trim().to_string()
    };

    let repo = read_input("GitHub repository URL");
    if repo.is_empty() {
        return Err("Repository URL is required".to_string());
    }

    let guessed = guess_name(&repo);
    let name_input = read_input(&format!("Server name [{}]", guessed));
    let name = if name_input.is_empty() { guessed.clone() } else { name_input };

    let category = read_input("Category (ai, database, devops, productivity, etc.)");
    if category.is_empty() {
        return Err("Category is required".to_string());
    }

    let title_default = {
        let mut c = guessed.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().to_string() + c.as_str(),
        }
    };
    let title_input = read_input(&format!("Title [{}]", title_default));
    let title = if title_input.is_empty() { title_default } else { title_input };

    let description = read_input("Description");

    let server = Server {
        name: name.clone(),
        image: format!("mcp/{name}"),
        server_type: "server".to_string(),
        meta: Meta { category: category.clone(), tags: vec![category] },
        about: About { icon: String::new(), title, description },
        source: Source { project: repo, ..Default::default() },
        ..Default::default()
    };

    let server_dir = PathBuf::from("servers").join(&name);
    std::fs::create_dir_all(&server_dir).map_err(|e| format!("{e}"))?;
    let yaml = serde_yaml::to_string(&server).map_err(|e| format!("{e}"))?;
    let path = server_dir.join("server.yaml");
    std::fs::write(&path, &yaml).map_err(|e| format!("{e}"))?;

    println!("\nServer definition written to {}", path.display());
    println!("\nNext steps:");
    println!("  1. Review servers/{name}/server.yaml");
    println!("  2. mcp-registry build {name}");
    println!("  3. mcp-registry catalog {name}");

    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build { server, tools, pull_community } => {
            cmd_build(&server, tools, pull_community).await
        }
        Commands::Catalog { server } => cmd_catalog(&server),
        Commands::Create { url, name, category, image, no_build, no_tools, args } => {
            cmd_create(&url, name.as_deref(), &category, image.as_deref(), !no_build, !no_tools, &args).await
        }
        Commands::Validate { name } => cmd_validate(&name),
        Commands::Wizard => cmd_wizard(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
