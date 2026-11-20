//! Restricted HTTP/1 response framing. No decompression, redirect or retry.

use tokio::io::{AsyncRead, AsyncReadExt};

use super::InventoryError;

const MAX_HEAD: usize = 16_384;
const MAX_HEADERS: usize = 64;
const MAX_CHUNK_LINE: usize = 128;

pub(super) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

enum Framing {
    Length(usize),
    Chunked,
    UntilClosed,
}

fn framing(head: &[u8]) -> Result<(u16, Framing), InventoryError> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut response = httparse::Response::new(&mut headers);
    if !response
        .parse(head)
        .map_err(|_| InventoryError::InvalidResponse)?
        .is_complete()
    {
        return Err(InventoryError::InvalidResponse);
    }
    let code = response.code.ok_or(InventoryError::InvalidResponse)?;
    if (300..400).contains(&code) {
        return Err(InventoryError::RedirectRejected);
    }
    if code != 200 {
        return Err(InventoryError::InvalidResponse);
    }
    let mut length = None;
    let mut chunked = false;
    for header in response.headers {
        if header.name.eq_ignore_ascii_case("content-length") {
            if header.value.is_empty()
                || !header.value.iter().all(u8::is_ascii_digit)
                || length.is_some()
            {
                return Err(InventoryError::InvalidResponse);
            }
            length = Some(
                std::str::from_utf8(header.value)
                    .map_err(|_| InventoryError::InvalidResponse)?
                    .parse::<usize>()
                    .map_err(|_| InventoryError::InvalidResponse)?,
            );
        }
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !header.value.eq_ignore_ascii_case(b"chunked") {
                return Err(InventoryError::InvalidResponse);
            }
            chunked = true;
        }
        if header.name.eq_ignore_ascii_case("content-encoding")
            && !header.value.eq_ignore_ascii_case(b"identity")
        {
            return Err(InventoryError::InvalidResponse);
        }
    }
    if chunked && length.is_some() {
        return Err(InventoryError::InvalidResponse);
    }
    Ok((
        code,
        if chunked {
            Framing::Chunked
        } else {
            length.map_or(Framing::UntilClosed, Framing::Length)
        },
    ))
}

async fn read_line<S: AsyncRead + Unpin + ?Sized>(
    stream: &mut S,
    bound: usize,
) -> Result<Vec<u8>, InventoryError> {
    let mut bytes = Vec::new();
    loop {
        let byte = stream
            .read_u8()
            .await
            .map_err(|_| InventoryError::InvalidResponse)?;
        bytes.push(byte);
        if bytes.len() > bound {
            return Err(InventoryError::InvalidResponse);
        }
        if bytes.ends_with(b"\r\n") {
            bytes.truncate(bytes.len() - 2);
            return Ok(bytes);
        }
    }
}

async fn exact_bytes<S: AsyncRead + Unpin + ?Sized>(
    stream: &mut S,
    body: &mut Vec<u8>,
    length: usize,
    bound: usize,
) -> Result<(), InventoryError> {
    let end = body
        .len()
        .checked_add(length)
        .filter(|end| *end <= bound)
        .ok_or(InventoryError::ResponseTooLarge)?;
    body.try_reserve(length)
        .map_err(|_| InventoryError::Unavailable)?;
    let start = body.len();
    body.resize(end, 0);
    stream
        .read_exact(
            body.get_mut(start..end)
                .ok_or(InventoryError::InvalidResponse)?,
        )
        .await
        .map_err(|_| InventoryError::InvalidResponse)?;
    Ok(())
}

async fn chunks<S: AsyncRead + Unpin + ?Sized>(
    stream: &mut S,
    bound: usize,
) -> Result<Vec<u8>, InventoryError> {
    let mut body = Vec::new();
    loop {
        let line = read_line(stream, MAX_CHUNK_LINE).await?;
        // Extensions and trailers are outside this first qualified dialect.
        if line.is_empty() || !line.iter().all(u8::is_ascii_hexdigit) {
            return Err(InventoryError::InvalidResponse);
        }
        let length = usize::from_str_radix(
            std::str::from_utf8(&line).map_err(|_| InventoryError::InvalidResponse)?,
            16,
        )
        .map_err(|_| InventoryError::InvalidResponse)?;
        if length == 0 {
            if !read_line(stream, MAX_HEAD).await?.is_empty() {
                return Err(InventoryError::InvalidResponse);
            }
            return Ok(body);
        }
        exact_bytes(stream, &mut body, length, bound).await?;
        let mut delimiter = [0; 2];
        stream
            .read_exact(&mut delimiter)
            .await
            .map_err(|_| InventoryError::InvalidResponse)?;
        if delimiter != *b"\r\n" {
            return Err(InventoryError::InvalidResponse);
        }
    }
}

pub(super) async fn read_response<S: AsyncRead + Unpin + ?Sized>(
    stream: &mut S,
    bound: usize,
) -> Result<Response, InventoryError> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(
            stream
                .read_u8()
                .await
                .map_err(|_| InventoryError::InvalidResponse)?,
        );
        if head.len() > MAX_HEAD {
            return Err(InventoryError::InvalidResponse);
        }
    }
    let (status, framing) = framing(&head)?;
    let body = match framing {
        Framing::Length(length) => {
            let mut body = Vec::new();
            exact_bytes(stream, &mut body, length, bound).await?;
            body
        }
        Framing::Chunked => chunks(stream, bound).await?,
        Framing::UntilClosed => {
            let mut body = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let length = stream
                    .read(&mut buffer)
                    .await
                    .map_err(|_| InventoryError::InvalidResponse)?;
                if length == 0 {
                    break;
                }
                let bytes = buffer
                    .get(..length)
                    .ok_or(InventoryError::InvalidResponse)?;
                if body
                    .len()
                    .checked_add(bytes.len())
                    .is_none_or(|end| end > bound)
                {
                    return Err(InventoryError::ResponseTooLarge);
                }
                body.extend_from_slice(bytes);
            }
            body
        }
    };
    Ok(Response { status, body })
}

#[cfg(test)]
mod tests {
    use super::read_response;
    use crate::inventory_transport::InventoryError;

    #[tokio::test]
    async fn t_f012_013_inventory_ambiguous_framing_never_yields_a_body() {
        for wire in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"
                .as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nContent-Length: 13\r\n\r\n{\"models\":[]}"
                .as_slice(),
        ] {
            let mut stream = wire;
            assert!(matches!(
                read_response(&mut stream, 128).await,
                Err(InventoryError::InvalidResponse)
            ));
        }
    }

    #[tokio::test]
    async fn t_f012_013_inventory_interrupted_framing_never_yields_a_body() {
        for wire in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\n\r\n{\"models\":[]}".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nd\r\n{\"models\":[]}".as_slice(),
        ] {
            let mut stream = wire;
            assert!(matches!(
                read_response(&mut stream, 128).await,
                Err(InventoryError::InvalidResponse)
            ));
        }
    }

    #[tokio::test]
    async fn t_f012_013_inventory_does_not_decompress_an_encoded_body() {
        let mut stream = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 13\r\n\r\n{\"models\":[]}".as_slice();
        assert!(matches!(
            read_response(&mut stream, 128).await,
            Err(InventoryError::InvalidResponse)
        ));
    }

    #[tokio::test]
    async fn t_f012_013_inventory_chunked_body_at_bound_is_complete() -> Result<(), InventoryError>
    {
        let mut stream = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nd\r\n{\"models\":[]}\r\n0\r\n\r\n".as_slice();
        let response = read_response(&mut stream, 13).await?;
        assert_eq!(response.status, 200);
        assert_eq!(response.body, br#"{"models":[]}"#);
        Ok(())
    }
}
