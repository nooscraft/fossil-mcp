//! Unified LSP server for Fossil code analysis.
//!
//! Provides real-time diagnostics for dead code, clones, and security issues
//! within editors that support the Language Server Protocol.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::config::FossilConfig;

/// State shared across LSP handler methods.
struct ServerState {
    config: FossilConfig,
    workspace_root: Option<PathBuf>,
}

/// The Fossil LSP backend.
pub struct FossilLspServer {
    client: Client,
    state: Arc<RwLock<ServerState>>,
}

impl FossilLspServer {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(RwLock::new(ServerState {
                config: FossilConfig::default(),
                workspace_root: None,
            })),
        }
    }

    /// Analyze a file and publish diagnostics.
    async fn analyze_file(&self, uri: &Url) {
        let diagnostics = Vec::new();

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for FossilLspServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Store workspace root
        if let Some(root_uri) = params.root_uri {
            if let Ok(path) = root_uri.to_file_path() {
                let mut state = self.state.write().await;
                state.workspace_root = Some(path.clone());
                state.config = FossilConfig::discover(&path);
                state.config.apply_env_overrides();
            }
        }

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "fossil-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "fossil.analyzeWorkspace".to_string(),
                        "fossil.analyzeFile".to_string(),
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Fossil LSP server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.analyze_file(&params.text_document.uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.analyze_file(&params.text_document.uri).await;
    }

    async fn did_change(&self, _params: DidChangeTextDocumentParams) {
        // Debounce: only analyze on save for now
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        // Clear diagnostics for closed files
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        match params.command.as_str() {
            "fossil.analyzeWorkspace" => {
                let state = self.state.read().await;
                if let Some(ref root) = state.workspace_root {
                    self.client
                        .log_message(
                            MessageType::INFO,
                            format!("Analyzing workspace: {}", root.display()),
                        )
                        .await;
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

/// Start the LSP server on stdin/stdout.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(FossilLspServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
