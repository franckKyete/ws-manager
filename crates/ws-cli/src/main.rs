use clap::{CommandFactory, Parser};
use std::process;
use ws_core::config::ConfigLoader;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

mod cli;
mod commands;
mod helpers;

use cli::*;
use commands::completion::*;
use commands::create::*;
use commands::daemon::*;
use commands::doctor::*;
use commands::end::*;
use commands::env::*;
use commands::exec::*;
use commands::focus::*;
use commands::git_ops::*;
use commands::hub::*;
use commands::info::*;
use commands::init::*;
use commands::launch::*;
use commands::list::*;
use commands::open::*;
use commands::project::*;
use commands::repo::*;
use commands::service::*;
use commands::setup::*;
use commands::status::*;
use helpers::normalize_cli_args;

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let normalized = normalize_cli_args(&raw_args);

    let mut full_args = vec![std::env::args().next().unwrap_or_else(|| "ws".to_string())];
    full_args.extend(normalized);

    let cli = match Cli::try_parse_from(&full_args) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            process::exit(if e.use_stderr() { 1 } else { 0 });
        }
    };

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            let mut cmd = Cli::command();
            let _ = cmd.print_help();
            println!();
            return;
        }
    };

    // 1. Fast paths for completion commands without loading workspace config
    match &command {
        Commands::Completion { shell, install } => {
            if let Err(e) = execute_completion(shell.as_deref(), *install) {
                OutputHandler::print_err(&e);
                process::exit(1);
            }
            return;
        }
        Commands::InternalComplete {
            query_type,
            query_args,
        } => {
            let target = query_args.first().map(|s| s.as_str());
            if let Err(e) = execute_internal_complete(query_type, target) {
                OutputHandler::print_err(&e);
                process::exit(1);
            }
            return;
        }
        _ => {}
    }

    // 2. Determine whether empty config is permitted
    let allow_empty_config = matches!(
        &command,
        Commands::Init { .. }
            | Commands::Add { .. }
            | Commands::Doctor
            | Commands::Antigravity
            | Commands::Project(_)
            | Commands::Hub(_)
            | Commands::Clone { .. }
            | Commands::Daemon { .. }
            | Commands::Service { .. }
    );

    let app_config = match ConfigLoader::load_config(
        cli.config.as_deref(),
        cli.workspaces_dir.as_deref(),
        allow_empty_config,
    ) {
        Ok(cfg) => cfg,
        Err(e) => {
            OutputHandler::print_err(&e.to_string());
            process::exit(1);
        }
    };

    let mut manager = WorkspaceManager::new(app_config, None);

    // 3. Dispatch to specific command implementation
    let result = match command {
        Commands::Create(args) => execute_create(
            &mut manager,
            args.name.as_deref(),
            args.file.as_deref(),
            args.setup,
            args.tmux_cmd.as_deref(),
            args.no_tmux,
            &args.extra_args,
            args.target_branch.as_deref(),
        ),
        Commands::List => execute_list(&manager),
        Commands::Info { name } => execute_info(&manager, name.as_deref()),
        Commands::Focus { name } => execute_focus(&manager, name.as_deref()),
        Commands::End(args) => execute_end(
            &manager,
            args.name.as_deref(),
            args.force,
            args.no_merge,
            args.delete_branch,
            args.target_branch.as_deref(),
            args.no_tmux,
        ),
        Commands::Status { name } => execute_status(&manager, name.as_deref()),
        Commands::Exec(args) => execute_exec(
            &manager,
            args.name.as_deref(),
            &args.command,
            args.all,
            args.repos_flag.as_deref(),
        ),
        Commands::Push(args) => execute_push(
            &manager,
            args.name.as_deref(),
            Some(&args.repos),
            args.repos_flag.as_deref(),
            Some(&args.remote),
        ),
        Commands::Pull(args) => execute_pull(
            &manager,
            args.name.as_deref(),
            Some(&args.repos),
            args.repos_flag.as_deref(),
            Some(&args.remote),
        ),
        Commands::Repo(args) => match args.action {
            Some(RepoAction::Add {
                name,
                repo,
                branch,
                existing,
            }) => execute_repo_add(
                &manager,
                name.as_deref(),
                repo.as_deref(),
                branch.as_deref(),
                existing,
            ),
            Some(RepoAction::Remove {
                name,
                repo,
                delete_branch,
            }) => execute_repo_remove(&manager, name.as_deref(), repo.as_deref(), delete_branch),
            Some(RepoAction::Lock { name, repo }) => {
                execute_repo_lock(&manager, name.as_deref(), repo.as_deref())
            }
            Some(RepoAction::Unlock { name, repo }) => {
                execute_repo_unlock(&manager, name.as_deref(), repo.as_deref())
            }
            None => {
                OutputHandler::print_err("Please specify a repo action: add, remove, lock, unlock");
                process::exit(1);
            }
        },
        Commands::Lock { name, repo } => {
            execute_repo_lock(&manager, name.as_deref(), repo.as_deref())
        }
        Commands::Unlock { name, repo } => {
            execute_repo_unlock(&manager, name.as_deref(), repo.as_deref())
        }
        Commands::Start(args) => {
            let mut mode = args.mode;
            if args.zellij {
                mode = Some("zellij".to_string());
            } else if args.tmux {
                mode = Some("tmux".to_string());
            } else if args.terminal {
                mode = Some("terminal".to_string());
            } else if args.stream {
                mode = Some("stream".to_string());
            } else if args.attach.is_some() {
                mode = Some("attach".to_string());
            }

            execute_start(
                &mut manager,
                args.name.as_deref(),
                Some(&args.repos),
                args.all,
                args.repos_flag.as_deref(),
                mode.as_deref(),
                args.attach.as_deref(),
                args.daemon,
                args.switch,
                args.interface.as_deref(),
                args.lan_ip.as_deref(),
            )
        }
        Commands::Attach(args) => {
            let mut mode = args.mode;
            if args.zellij {
                mode = Some("zellij".to_string());
            } else if args.tmux {
                mode = Some("tmux".to_string());
            }

            execute_attach(
                &manager,
                args.name.as_deref(),
                args.repo.as_deref(),
                args.all,
                mode.as_deref(),
                args.switch,
            )
        }
        Commands::Stop { name } => execute_stop(&manager, name.as_deref()),
        Commands::Restart { name, repos } => {
            execute_restart(&mut manager, name.as_deref(), None, Some(&repos))
        }
        Commands::Logs(args) => execute_logs(
            &manager,
            args.name.as_deref(),
            args.repo.as_deref(),
            args.follow,
            args.lines,
        ),
        Commands::Bridge { name, repo } => {
            execute_bridge(&manager, name.as_deref(), repo.as_deref())
        }
        Commands::Shell { name, worktree } => {
            execute_open(&manager, name.as_deref(), worktree.as_deref())
        }
        Commands::Env(args) => execute_env(
            &mut manager,
            args.name.as_deref(),
            args.repo.as_deref(),
            args.sync,
            args.interface.as_deref(),
            args.lan_ip.as_deref(),
        ),
        Commands::Setup(args) => execute_setup(
            &mut manager,
            args.name.as_deref(),
            Some(&args.repos),
            args.all,
            args.repos_flag.as_deref(),
            args.dry_run,
            args.skip_scripts,
            cli.verbose,
            args.interface.as_deref(),
            args.lan_ip.as_deref(),
        ),
        Commands::Project(args) => match args.action {
            Some(ProjectAction::Init { urls }) => execute_init(&mut manager, &urls),
            Some(ProjectAction::Add { url }) => execute_add(&mut manager, &url),
            Some(ProjectAction::Fetch) => execute_fetch(&manager),
            Some(ProjectAction::Sync) => execute_sync(&manager),
            Some(ProjectAction::List) => execute_project_list(),
            Some(ProjectAction::Register { path }) => execute_project_register(path.as_deref()),
            Some(ProjectAction::Unregister { path }) => execute_project_unregister(path.as_deref()),
            None => {
                OutputHandler::print_err(
                    "Please specify a project action: init, add, fetch, sync, list, register, unregister",
                );
                process::exit(1);
            }
        },
        Commands::Init { urls } => execute_init(&mut manager, &urls),
        Commands::Add { url } => execute_add(&mut manager, &url),
        Commands::Fetch => execute_fetch(&manager),
        Commands::Sync => execute_sync(&manager),
        Commands::Doctor => execute_doctor(&manager),
        Commands::Antigravity => execute_antigravity(&manager),
        Commands::Clone {
            project,
            target_dir,
        } => execute_hub_clone(&project, target_dir.as_deref()),
        Commands::Hub(args) => match args.action {
            Some(HubAction::Login {
                url,
                token,
                username,
                password,
            }) => execute_hub_login(
                url.as_deref(),
                token.as_deref(),
                username.as_deref(),
                password.as_deref(),
            ),
            Some(HubAction::Whoami) => execute_hub_whoami(),
            Some(HubAction::Logout) => execute_hub_logout(),
            Some(HubAction::Clone {
                project,
                target_dir,
            }) => execute_hub_clone(&project, target_dir.as_deref()),
            Some(HubAction::Publish {
                project,
                description,
            }) => execute_hub_publish(&mut manager, project.as_deref(), description.as_deref()),
            Some(HubAction::Push { message, project }) => {
                let _ = project;
                execute_hub_push(&mut manager, Some(&message), false)
            }
            Some(HubAction::Pull { project }) => execute_hub_pull(&mut manager, project.as_deref()),
            Some(HubAction::Status { .. }) => execute_hub_status(&manager),
            Some(HubAction::Sync { project }) => execute_hub_sync(&mut manager, project.as_deref()),
            Some(HubAction::State { action }) => match action {
                HubStateAction::Save {
                    workspace,
                    project: _,
                    no_wip: _,
                    auto: _,
                } => execute_hub_state_save(&manager, Some(&workspace), None),
                HubStateAction::Restore {
                    workspace,
                    project: _,
                    no_wip: _,
                } => execute_hub_state_restore(&mut manager, Some(&workspace)),
            },
            Some(HubAction::Resume {
                workspace,
                project: _,
                no_wip: _,
            }) => execute_hub_state_restore(&mut manager, Some(&workspace)),
            Some(HubAction::AutoSave { action }) => match action {
                Some(HubAutoSaveAction::Status) | None => execute_hub_auto_save_status(&manager),
                Some(HubAutoSaveAction::Start {
                    interval: _,
                    project: _,
                    daemon,
                }) => execute_hub_auto_save_start(&manager, None, None, daemon),
                Some(HubAutoSaveAction::Stop) => execute_hub_auto_save_stop(&manager),
                Some(HubAutoSaveAction::Run { .. }) | Some(HubAutoSaveAction::Once { .. }) => {
                    execute_hub_auto_save_run(&manager)
                }
                Some(HubAutoSaveAction::Daemon { tick }) => execute_daemon(Some(tick)),
                Some(HubAutoSaveAction::Service { action }) => dispatch_service_action(action),
            },
            Some(HubAction::Secret { action }) => match action {
                HubSecretAction::List { project: _ } => execute_hub_secret_list(&manager),
                HubSecretAction::Set {
                    key,
                    value,
                    repo,
                    project: _,
                } => execute_hub_secret_set(&manager, &key, &value, repo.as_deref()),
                HubSecretAction::Get {
                    key,
                    repo,
                    project: _,
                } => execute_hub_secret_get(&manager, &key, repo.as_deref()),
                HubSecretAction::Delete {
                    key,
                    repo,
                    project: _,
                } => execute_hub_secret_delete(&manager, &key, repo.as_deref()),
                HubSecretAction::Upload { file_path, project } => {
                    execute_hub_secret_upload(&manager, &file_path, project.as_deref())
                }
                HubSecretAction::Pull { project } => {
                    execute_hub_secret_pull(&manager, project.as_deref())
                }
            },
            None => {
                let mut cmd = Cli::command();
                let _ = cmd.print_help();
                println!();
                Ok(())
            }
        },
        Commands::Daemon { tick, action: _ } => execute_daemon(Some(tick)),
        Commands::Service { action } => dispatch_service_action(action),
        Commands::Completion { .. } | Commands::InternalComplete { .. } => unreachable!(),
    };

    if let Err(err) = result {
        OutputHandler::print_err(&err);
        process::exit(1);
    }
}

fn dispatch_service_action(action: Option<ServiceAction>) -> Result<(), String> {
    match action {
        Some(ServiceAction::Install) => execute_service_install(None),
        Some(ServiceAction::Uninstall) => execute_service_uninstall(),
        Some(ServiceAction::Start) => execute_service_control("start"),
        Some(ServiceAction::Stop) => execute_service_control("stop"),
        Some(ServiceAction::Restart) => execute_service_control("restart"),
        Some(ServiceAction::Enable) => execute_service_control("enable"),
        Some(ServiceAction::Disable) => execute_service_control("disable"),
        Some(ServiceAction::Status) | None => execute_service_status(),
        Some(ServiceAction::Logs { follow, lines }) => execute_service_logs(follow, lines),
    }
}
