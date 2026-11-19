use std::io;
use std::path::Path;

use base64::Engine;

use lsp_types::*;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_HEADER: usize = 8 * 1024;
const MAX_BODY: usize = 64 * 1024 * 1024;

pub(crate) async fn read(reader: &mut (impl AsyncBufRead + Unpin)) -> io::Result<Value> {
    let mut size = None;
    let mut consumed = 0;
    loop {
        let mut line = String::new();
        let count = reader.read_line(&mut line).await?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Tinymist closed stdout",
            ));
        }
        consumed += count;
        if consumed > MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP headers exceed 8 KiB",
            ));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            if size.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate LSP Content-Length",
                ));
            }
            size = Some(value.trim().parse::<usize>().map_err(io::Error::other)?);
        }
    }
    let size = size
        .filter(|size| (1..=MAX_BODY).contains(size))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid LSP Content-Length"))?;
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub(crate) async fn write(
    writer: &mut (impl AsyncWrite + Unpin),
    message: &Value,
) -> io::Result<()> {
    let json = serde_json::to_vec(message).map_err(io::Error::other)?;
    if json.len() > MAX_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LSP message exceeds 64 MiB",
        ));
    }
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", json.len()).as_bytes())
        .await?;
    writer.write_all(&json).await?;
    writer.flush().await
}

pub(crate) fn notice(method: &str, params: impl serde::Serialize) -> Value {
    json!({"jsonrpc":"2.0", "method":method, "params":params})
}

pub(crate) fn request(id: u64, method: &str, params: impl serde::Serialize) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

pub(crate) fn response_error(message: &Value) -> Option<String> {
    message.get("error").map(|error| {
        error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string())
    })
}

macro_rules! queries {
    ($($kind:ident($params:ty) -> $result:ty = $method:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Kind { $($kind),* }

        pub enum Query {
            $($kind($params)),*
        }

        pub enum Reply {
            $($kind($result)),*
        }

        impl Query {
            pub fn kind(&self) -> Kind {
                match self { $(Self::$kind(_) => Kind::$kind),* }
            }

            pub(crate) fn encode(self, id: u64) -> Value {
                match self { $(Self::$kind(params) => request(id, $method, params)),* }
            }
        }

        impl Kind {
            pub(crate) fn decode(self, result: Value) -> Result<Reply, String> {
                match self {
                    $(Self::$kind => serde_json::from_value(result).map(Reply::$kind)
                        .map_err(|error| format!("Invalid {} reply: {error}", $method))),*
                }
            }
        }
    };
}

queries! {
    Completion(CompletionParams) -> Option<CompletionResponse> = "textDocument/completion",
    Hover(HoverParams) -> Option<Hover> = "textDocument/hover",
    Signature(SignatureHelpParams) -> Option<SignatureHelp> = "textDocument/signatureHelp",
    Definition(GotoDefinitionParams) -> Option<GotoDefinitionResponse> = "textDocument/definition",
    References(ReferenceParams) -> Option<Vec<Location>> = "textDocument/references",
    Rename(RenameParams) -> Option<WorkspaceEdit> = "textDocument/rename",
    Format(DocumentFormattingParams) -> Option<Vec<TextEdit>> = "textDocument/formatting",
    Outline(DocumentSymbolParams) -> Option<DocumentSymbolResponse> = "textDocument/documentSymbol",
    Folding(FoldingRangeParams) -> Option<Vec<FoldingRange>> = "textDocument/foldingRange",
    Semantic(SemanticTokensParams) -> Option<SemanticTokensResult> = "textDocument/semanticTokens/full",
    Inlay(InlayHintParams) -> Option<Vec<InlayHint>> = "textDocument/inlayHint",
    Actions(CodeActionParams) -> Option<CodeActionResponse> = "textDocument/codeAction",
    Execute(ExecuteCommandParams) -> Value = "workspace/executeCommand",
    Export(ExecuteCommandParams) -> ExportData = "workspace/executeCommand",
}

#[derive(serde::Deserialize)]
pub struct ExportData {
    #[serde(deserialize_with = "decode_export_data")]
    data: Vec<u8>,
}

impl ExportData {
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

fn decode_export_data<'de, D>(decoder: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let encoded = <String as serde::Deserialize>::deserialize(decoder)?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(serde::de::Error::custom)
}

impl Query {
    pub fn export_text(main: &Path, markdown: bool) -> Self {
        Self::Export(ExecuteCommandParams {
            command: if markdown {
                "tinymist.exportMarkdown"
            } else {
                "tinymist.exportText"
            }
            .into(),
            arguments: vec![json!(main), json!({}), json!({"write":false})],
            work_done_progress_params: Default::default(),
        })
    }
    pub fn pin(path: Option<String>) -> Self {
        Self::Execute(ExecuteCommandParams {
            command: "tinymist.pinMain".into(),
            arguments: vec![json!(path)],
            work_done_progress_params: Default::default(),
        })
    }

    pub fn pin_target(&self) -> Option<Option<String>> {
        let Self::Execute(params) = self else {
            return None;
        };
        if params.command != "tinymist.pinMain" {
            return None;
        }
        serde_json::from_value(params.arguments.first()?.clone()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn framing_round_trips_utf8_and_keeps_message_boundaries() -> io::Result<()> {
        let first = json!({"label":"🌱"});
        let second = json!({"id":2});
        let mut stream = Vec::new();
        write(&mut stream, &first).await?;
        write(&mut stream, &second).await?;
        let mut reader = BufReader::new(stream.as_slice());
        assert_eq!(read(&mut reader).await?, first);
        assert_eq!(read(&mut reader).await?, second);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_duplicate_and_oversized_content_length() {
        for data in [
            &b"Content-Length: 2\r\ncontent-length: 2\r\n\r\n{}"[..],
            &b"Content-Length: 9999999999\r\n\r\n"[..],
        ] {
            assert_eq!(
                read(&mut BufReader::new(data)).await.map_err(|e| e.kind()),
                Err(io::ErrorKind::InvalidData)
            );
        }
    }
}
