//! A single explicit binary command. Raw bytes are read once, bounded, and never
//! rendered. A transport error retains the original command identity for receipt
//! inspection; no branch retries, acquires control or guesses another sequence.

use serde_json::{Value, json};
use std::io::{IsTerminal, Read};
use std::path::Path;
use unleash_client::UnleashClient;
use unleash_domain::{AuthorityGeneration, ProjectId};
use unleash_protocol::CommandEnvelope;

use super::{TerminalInputArgs, send_terminal_intention};
use crate::CliError;

fn bounds_error() -> CliError {
    CliError::InvalidArgument("Terminal input must contain 1..=65536 bytes".into())
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    // Read one extra byte to detect overflow, including a file that grows after
    // metadata inspection. Never drain an unbounded input into memory.
    reader.take(65_537).read_to_end(&mut bytes)?;
    if !(1..=65_536).contains(&bytes.len()) {
        return Err(bounds_error());
    }
    Ok(bytes)
}

fn read_input(path: &Path) -> Result<Vec<u8>, CliError> {
    if path == Path::new("-") {
        let stdin = std::io::stdin();
        if stdin.is_terminal() {
            return Err(CliError::InvalidArgument(
                "Binary input requires a file or non-interactive stdin".into(),
            ));
        }
        return read_bounded(stdin.lock());
    }
    if !std::fs::metadata(path)?.is_file() {
        return Err(CliError::InvalidArgument(
            "Binary input requires a regular file".into(),
        ));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(CliError::InvalidArgument(
            "Binary input file changed type".into(),
        ));
    }
    read_bounded(file)
}

fn envelope(
    args: &TerminalInputArgs,
    current: AuthorityGeneration,
    bytes: &[u8],
) -> Result<CommandEnvelope, CliError> {
    let mut command = args.control.envelope(current, "SendTerminalInput")?;
    command
        .payload
        .insert("input_seq".into(), json!(args.input_seq));
    command.payload.insert("payload_bytes".into(), json!(bytes));
    Ok(command)
}

pub(super) async fn send(
    client: &mut UnleashClient,
    args: &TerminalInputArgs,
) -> Result<Value, CliError> {
    let bytes = read_input(&args.file)?;
    let command = envelope(args, client.hello().authority_generation, &bytes)?;
    send_terminal_intention(
        client,
        ProjectId::from_uuid(args.control.project_id),
        &command,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_project_is_forwarded_to_writer() {
        let args = TerminalInputArgs {
            control: super::super::TerminalControlArgs {
                project_id: uuid::Uuid::nil(),
                terminal_id: uuid::Uuid::from_u128(1),
                process_run_uuid: uuid::Uuid::from_u128(2),
                lease_id: uuid::Uuid::from_u128(3),
                fence: 1,
                control_revision: 0,
                command_id: None,
                authority_generation: None,
            },
            input_seq: 1,
            file: std::path::PathBuf::new(),
        };
        let command = envelope(
            &args,
            AuthorityGeneration::from_uuid(uuid::Uuid::from_u128(4)),
            &[0, 255],
        );
        assert_eq!(
            command.map(|value| value.project_id).map_err(|_| ()),
            Ok(Some(ProjectId::from_uuid(uuid::Uuid::nil()))),
            "a provided UUID project is resolved by the writer, including nil"
        );
    }

    #[test]
    fn binary_reader_preserves_arbitrary_bytes_and_stops_at_limit_plus_one() {
        for bytes in [vec![0, 255, 0, 10], vec![0x80; 65_536]] {
            assert_eq!(
                read_bounded(bytes.as_slice()).ok().as_deref(),
                Some(bytes.as_slice())
            );
        }
        assert!(matches!(
            read_bounded(&[][..]),
            Err(CliError::InvalidArgument(_))
        ));
        let mut oversized = std::io::Cursor::new(vec![0; 65_538]);
        assert!(matches!(
            read_bounded(&mut oversized),
            Err(CliError::InvalidArgument(_))
        ));
        assert_eq!(
            oversized.position(),
            65_537,
            "stop at the first overflow byte"
        );
    }
}
