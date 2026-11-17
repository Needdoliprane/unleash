//! Tests du codec de trames Unix IPC (synchrone, u32 big-endian + UTF-8, plafond strict 8 MiB).

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use std::io::Cursor;
use unleash_protocol::{FrameBuffer, MAX_FRAME_BYTES, ProtocolError, read_frame, write_frame};

#[test]
fn test_frame_roundtrip() {
    let payload = b"{\"hello\":\"world\",\"protocol\":\"unleash.ipc.v2\"}";
    let mut buffer = Vec::new();

    write_frame(&mut buffer, payload).expect("write_frame doit réussir");
    assert_eq!(buffer.len(), 4 + payload.len());

    let mut cursor = Cursor::new(buffer);
    let read_payload = read_frame(&mut cursor).expect("read_frame doit réussir");
    assert_eq!(read_payload, Some(payload.to_vec()));

    // EOF propre à la frontière de trame
    let eof_result = read_frame(&mut cursor).expect("EOF propre");
    assert_eq!(eof_result, None);
}

#[test]
fn test_frame_exceeding_8mib_rejected_before_allocation() {
    // Cas 1 : En-tête annonçant 8_388_609 octets (8 MiB + 1 octet)
    let bad_length = (MAX_FRAME_BYTES as u32) + 1;
    let mut data = bad_length.to_be_bytes().to_vec();
    // On n'ajoute pas 8 MiB de données pour s'assurer que l'erreur survient AVANT lecture/allocation !
    data.extend_from_slice(b"small dummy payload");

    let mut cursor = Cursor::new(data);
    let result = read_frame(&mut cursor);
    match result {
        Err(ProtocolError::FrameTooLarge { length, max }) => {
            assert_eq!(length, bad_length);
            assert_eq!(max, MAX_FRAME_BYTES as u32);
        }
        other => panic!("Attendu ProtocolError::FrameTooLarge, obtenu : {other:?}"),
    }

    // Cas 2 : En-tête annonçant 1 GiB (tentative d'attaque DoS par épuisement mémoire)
    let attack_length = 1024 * 1024 * 1024_u32;
    let attack_data = attack_length.to_be_bytes().to_vec();
    let mut cursor_attack = Cursor::new(attack_data);
    let result_attack = read_frame(&mut cursor_attack);
    assert!(matches!(
        result_attack,
        Err(ProtocolError::FrameTooLarge { length, .. }) if length == attack_length
    ));
}

#[test]
fn test_write_frame_exceeding_8mib_rejected() {
    // Tentative d'écriture d'une trame > 8 MiB
    let oversized = vec![0u8; MAX_FRAME_BYTES + 1];
    let mut sink = Vec::new();
    let res = write_frame(&mut sink, &oversized);
    assert!(matches!(res, Err(ProtocolError::FrameTooLarge { .. })));
    assert!(
        sink.is_empty(),
        "Aucun octet ne doit être écrit si la taille excède 8 MiB"
    );
}

#[test]
fn test_read_frame_unexpected_eof() {
    // En-tête annonce 100 octets, mais seulement 10 octets sont disponibles
    let mut data = 100_u32.to_be_bytes().to_vec();
    data.extend_from_slice(b"0123456789");

    let mut cursor = Cursor::new(data);
    let res = read_frame(&mut cursor);
    assert!(matches!(res, Err(ProtocolError::UnexpectedEof)));
}

#[test]
fn test_frame_buffer_fragmentation_and_pipelining() {
    let mut fb = FrameBuffer::new();

    let frame1 = b"{\"cmd\":\"First\"}";
    let frame2 = b"{\"cmd\":\"Second\"}";

    let mut encoded1 = Vec::new();
    write_frame(&mut encoded1, frame1).unwrap();

    let mut encoded2 = Vec::new();
    write_frame(&mut encoded2, frame2).unwrap();

    // 1. Pousser des données fragmentées octet par octet pour frame 1
    for byte in &encoded1[..5] {
        fb.push_bytes(&[*byte]);
        assert_eq!(fb.next_frame().unwrap(), None);
    }
    for byte in &encoded1[5..encoded1.len() - 1] {
        fb.push_bytes(&[*byte]);
        assert_eq!(fb.next_frame().unwrap(), None);
    }
    // Dernier octet de frame 1
    fb.push_bytes(&[encoded1[encoded1.len() - 1]]);
    let extracted1 = fb.next_frame().unwrap().expect("Frame 1 prête");
    assert_eq!(extracted1, frame1);
    assert_eq!(fb.next_frame().unwrap(), None);

    // 2. Pousser toute la frame 2 d'un coup
    fb.push_bytes(&encoded2);
    let extracted2 = fb.next_frame().unwrap().expect("Frame 2 prête");
    assert_eq!(extracted2, frame2);
    assert!(fb.is_empty());
}
