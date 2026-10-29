//! Strict single-range subset of HTTP. No URL or response-body diagnostics.
use super::*;

fn one(headers: &HeaderMap, name: header::HeaderName) -> Result<&HeaderValue, RemoteMediaError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or(RemoteMediaError::UnsupportedRange)?;
    if values.next().is_some() {
        return Err(RemoteMediaError::InvalidResponse);
    }
    Ok(value)
}
fn decimal(text: &str) -> Result<u64, RemoteMediaError> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RemoteMediaError::InvalidResponse);
    }
    text.parse().map_err(|_| RemoteMediaError::InvalidResponse)
}
pub(super) fn validate_headers(
    status: u16,
    headers: &HeaderMap,
    range: (u64, usize),
    expected: Option<&Representation>,
) -> Result<Representation, RemoteMediaError> {
    match status {
        206 => {}
        301 | 302 | 303 | 307 | 308 => return Err(RemoteMediaError::Redirected),
        401 | 403 | 404 => return Err(RemoteMediaError::AccessDenied),
        412 | 416 if expected.is_some() => return Err(RemoteMediaError::SourceChanged),
        429 => return Err(RemoteMediaError::RateLimited),
        500..=599 => return Err(RemoteMediaError::Network),
        _ => return Err(RemoteMediaError::UnsupportedRange),
    }
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return Err(RemoteMediaError::UnsupportedRange);
    }
    if headers.contains_key(header::CONTENT_TYPE)
        && one(headers, header::CONTENT_TYPE)?
            .to_str()
            .map_err(|_| RemoteMediaError::InvalidResponse)?
            .split(';')
            .next()
            .is_some_and(|mime| {
                mime.trim()
                    .as_bytes()
                    .get(..10)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"multipart/"))
            })
    {
        return Err(RemoteMediaError::UnsupportedRange);
    }
    if headers.contains_key(header::CONTENT_ENCODING)
        && !one(headers, header::CONTENT_ENCODING)?
            .as_bytes()
            .eq_ignore_ascii_case(b"identity")
    {
        return Err(RemoteMediaError::UnsupportedRange);
    }
    let etag = one(headers, header::ETAG)?;
    let tag = etag.as_bytes();
    if !(2..=MAX_ETAG_BYTES).contains(&tag.len())
        || tag.first() != Some(&b'"')
        || tag.last() != Some(&b'"')
        || !tag[1..tag.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte))
    {
        return Err(RemoteMediaError::UnsupportedRange);
    }
    let value = one(headers, header::CONTENT_RANGE)?
        .to_str()
        .map_err(|_| RemoteMediaError::InvalidResponse)?;
    let (unit, rest) = value
        .split_once(' ')
        .ok_or(RemoteMediaError::InvalidResponse)?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return Err(RemoteMediaError::UnsupportedRange);
    }
    let (span, total) = rest
        .split_once('/')
        .ok_or(RemoteMediaError::InvalidResponse)?;
    let (first, last) = span
        .split_once('-')
        .ok_or(RemoteMediaError::InvalidResponse)?;
    let (first, last, total) = (decimal(first)?, decimal(last)?, decimal(total)?);
    if total > MAX_REMOTE_MEDIA_BYTES {
        return Err(RemoteMediaError::TooLarge);
    }
    if first > last || last >= total {
        return Err(RemoteMediaError::InvalidResponse);
    }
    if let Some(expected) = expected
        && (expected.length != total || expected.etag != *etag)
    {
        return Err(RemoteMediaError::SourceChanged);
    }
    let wanted_last = range
        .0
        .checked_add(range.1 as u64)
        .and_then(|end| end.checked_sub(1))
        .ok_or(RemoteMediaError::InvalidRequest)?;
    let length = one(headers, header::CONTENT_LENGTH)?
        .to_str()
        .map_err(|_| RemoteMediaError::InvalidResponse)?;
    if first != range.0 || last != wanted_last || decimal(length)? != range.1 as u64 {
        return Err(RemoteMediaError::InvalidResponse);
    }
    Ok(Representation {
        length: total,
        etag: etag.clone(),
    })
}
