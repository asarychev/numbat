use anyhow::Result;
use clap::Parser;
use juker::{
    ConnectionInfo, JuHelpLink, JuKernel, JuKernelInfo,
    message::{EvalResult, EvalValue},
    server::JuServer,
};
use numbat::{
    Context, InterpreterSettings,
    diagnostic::ErrorDiagnostic,
    module_importer::{BuiltinModuleImporter, ChainedImporter, FileSystemImporter},
    resolver::CodeSource,
};
use numbat::{
    NumbatError::{NameResolutionError, ResolverError, RuntimeError, TypeCheckError},
    markup as m,
};
use serde_json::{Value, json};
use std::{env, fs::File, ops::Deref, path::PathBuf};
use tracing::{debug, error, info, level_filters::LevelFilter, trace, warn};
use tracing_subscriber::EnvFilter;
use tracing_udp::UdpTracingWriter;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct JupyterApplication {
    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
    #[arg(short = 'C', long)]
    connection_file: PathBuf,
    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,
    // #[command(subcommand)]
    // command: JupyterCommands,
}

// #[derive(Subcommand)]
// enum JupyterCommands {
//     Open(Box<OpenAction>),
//     Start(Box<StartAction>),
//     Install(Box<InstallAction>),
//     Uninstall(Box<UninstallAction>),
// }

impl JupyterApplication {
    pub async fn run(&self) -> Result<()> {
        error!("Error log example");
        warn!("Warning log example");
        info!("Info log example");
        debug!("Debug log example");
        trace!("Trace log example");

        let f = File::open(&self.connection_file)?;
        info!("Opened connection file: {:?}", f);

        let ci: ConnectionInfo = serde_json::from_reader(f)?;
        info!("Connection file content: {:?}", ci);

        loop {
            let eva = Eva::new()?;
            let res = JuServer::start(&ci, eva).await;

            match &res {
                Ok(true) => {
                    info!("Server exited and requested restart, restarting.");
                    continue;
                }
                Ok(false) => {
                    info!("Server exited successfully.");
                }
                Err(e) => {
                    error!("Server error: {:?}", e);
                }
            }
            break;
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::DEBUG.into())
                .from_env_lossy(),
        )
        .with_writer(UdpTracingWriter::new("localhost:5555")?)
        // Use a more compact, abbreviated log format
        .compact()
        // Display source code file paths
        .with_file(true)
        // Display source code line numbers
        .with_line_number(true)
        // // Display the thread ID an event was recorded on
        // .with_thread_ids(true)
        // // Don't display the event's target (module path)
        // .with_target(false)
        // // Build the subscriber
        .finish();
    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber)?;
    // let (non_blocking, _guard) = tracing_appender::non_blocking(std::io::stdout());

    // tracing_subscriber
    //     ::registry()
    //     .with(fmt::layer().with_writer(non_blocking))
    //     .with(
    //         EnvFilter::builder().with_default_directive(LevelFilter::DEBUG.into()).from_env_lossy()
    //     )
    //     .init();

    let args: Vec<String> = env::args().collect();

    debug!("args: {args:?}");

    let app = JupyterApplication::parse();
    let res = app.run().await;
    match &res {
        Ok(_) => {
            error!("Application exited successfully");
        }
        Err(e) => {
            error!("Application error: {:?}, bt:\n{:?}", e, e.backtrace());
        }
    };

    res
}

struct Eva {
    ctx: Context,
    settings: InterpreterSettings,
}

impl Eva {
    fn new() -> Result<Self> {
        let ctx = Self::make_fresh_context()?;
        let settings = InterpreterSettings {
            print_fn: Box::new(move |s: &m::Markup| {
                // to_be_printed_c.lock().unwrap().push(s.clone());
            }),
        };

        Ok(Eva { ctx, settings })
    }

    fn make_fresh_context() -> Result<Context> {
        let fs_importer = FileSystemImporter::default();
        // for path in Self::get_modules_paths() {
        //     fs_importer.add_path(path);
        // }

        let importer = ChainedImporter::new(
            Box::new(fs_importer),
            Box::<BuiltinModuleImporter>::default(),
        );

        let mut context = Context::new(importer);
        let _ = context.interpret("use prelude", CodeSource::Internal)?;

        Ok(context)
    }
}

fn tb<T: ErrorDiagnostic>(err: &T) -> Vec<Value> {
    err.diagnostics()
        .into_iter()
        .map(|d| format!("{}: {}", d.code.unwrap_or("".into()), d.message).into())
        .collect()
}

impl JuKernel for Eva {
    fn kernel_info(&self) -> JuKernelInfo {
        JuKernelInfo {
            name: "Numbat".to_string(),
            version: "0.0.0".to_string(),
            mimetype: "text/numbat".to_string(),
            file_extension: ".numbat".to_string(),
            banner: "Numbat Kernel".to_string(),
            help_links: vec![JuHelpLink {
                text: "Numbat Documentation".to_string(),
                url: "https://github.com/asaryche/juker".to_string(),
            }],
        }
    }

    async fn eval_code(&mut self, code: String) -> EvalResult {
        match self
            .ctx
            .interpret_with_settings(&mut self.settings, &code, CodeSource::Text)
        {
            Ok((_statements, res)) => {
                let txt = res.value_as_string().unwrap_or("".into());
                EvalResult::Success {
                    results: vec![EvalValue {
                        data: json!({
                            "text/plain": txt,
                        }),
                        metadata: json!({}),
                    }],
                }
            }
            Err(err) => {
                error!("Numbat error: {err:?}");
                let evalue = json!(err.to_string());

                let (ename, traceback) = match *err {
                    ResolverError(resolver_error) => ("ResolverError", tb(&resolver_error)),
                    NameResolutionError(name_resolution_error) => {
                        ("NameResolutionError", tb(&name_resolution_error))
                    }
                    TypeCheckError(type_check_error) => {
                        ("TypeCheckError", tb(&type_check_error))
                    },
                    RuntimeError(runtime_error) => (
                        "RuntimeError",
                        runtime_error
                            .backtrace
                            .into_iter()
                            .map(|(s, _)| json!(s))
                            .collect(),
                    ),
                };
                let ename = json!(ename);

                // TODO: better diagnostics
                let traceback = vec![ename.clone(), evalue.clone()];

                EvalResult::Error {
                    ename,
                    evalue,
                    traceback,
                }
            }
        }
    }
}

// use anyhow::Result;
// use numbat::{
//     Context, InterpreterSettings,
//     command::{CommandControlFlow, CommandRunner},
//     module_importer::{BuiltinModuleImporter, ChainedImporter, FileSystemImporter},
//     resolver::CodeSource,
// };

// fn main() -> Result<()> {
//     println!("Hello, world!");

//     let rl = &mut ();
//     let mut ctx = make_fresh_context();

//     let mut cmd_runner = CommandRunner::new()
//         // .print_with(|m| println!("{}", ansi_format(m, true)))
//         // .enable_clear(|rl| match rl.clear_screen() {
//         //     Ok(_) => CommandControlFlow::Continue,
//         //     Err(_) => CommandControlFlow::Return,
//         // })
//         // .enable_save(SessionHistory::default())
//         .enable_reset()
//         .enable_quit();

//     // let line = "let qq = 2 + 2 + 3; qq + 1";
//     let line = "help";
//     match cmd_runner.try_run_command(&line, &mut ctx, rl) {
//         Ok(cf) => match cf {
//             CommandControlFlow::Continue => {
//                 println!("Continue");
//             }
//             CommandControlFlow::Return => {
//                 println!("Return");
//             }
//             CommandControlFlow::Reset => {
//                 println!("Reset");
//                 todo!();
//             }
//             CommandControlFlow::NotACommand => {
//                 println!("NotACommand");
//             }
//         },
//         Err(err) => {
//             println!("Err: {err:?}");
//             // ctx.print_diagnostic(
//             //     ResolverDiagnostic {
//             //         resolver: ctx.resolver(),
//             //         error: &*err,
//             //     },
//             //     colored::control::SHOULD_COLORIZE.should_colorize(),
//             // );
//             // continue;
//         }
//     }

//     let mut settings = InterpreterSettings {
//         print_fn: Box::new(move |s: &m::Markup| {
//             // to_be_printed_c.lock().unwrap().push(s.clone());
//         }),
//     };

//     let (statements, interpretation_result) = ctx.interpret_with_settings(&mut settings, line, CodeSource::Text)?;
//     println!("Result: {:?}", interpretation_result);

//     for t in statements {
//         println!("Statement: {t:?}");
//     }

//     Ok(())
// }
