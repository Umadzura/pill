//! GET по HTTPS через WinHTTP (есть в каждой Windows, без зависимостей). Блокирует: только из воркера.
//! Нужен одной кнопке «Проверить обновление» (`feat::update`), фоном ничего не качаем.
//! Перенаправления (GitHub отдаёт файлы релиза с другого адреса) WinHTTP проходит сам, кроме HTTPS → HTTP.

use std::ffi::c_void;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

/// Части адреса. Принимаем только `https://`; `http://` — только для 127.0.0.1 (проверка обновления
/// на тестовом сервере, см. `feat::update::api_url`).
#[derive(Debug, PartialEq)]
pub struct Url {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(url: &str) -> Option<Url> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return None;
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (hostport, if secure { 443 } else { 80 }),
    };
    if host.is_empty() || host.contains('@') || (!secure && host != "127.0.0.1") {
        return None;
    }
    Some(Url { secure, host: host.to_string(), port, path: path.to_string() })
}

/// Закрывает дескриптор WinHTTP при выходе из области.
struct H(*mut c_void);

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// Скачать адрес целиком в память. Ошибка — короткий текст для окна настроек.
/// Ответ больше `max` байт обрывается ошибкой, тайм-аут 15 с на каждый шаг.
pub fn get(url: &str, accept: &str, max: usize) -> Result<Vec<u8>, String> {
    let u = parse_url(url).ok_or("неверный адрес")?;
    unsafe {
        let agent = HSTRING::from(format!("Pill/{}", env!("CARGO_PKG_VERSION")));
        let session = H(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err("WinHTTP не открылся".into());
        }
        let _ = WinHttpSetTimeouts(session.0, 15_000, 15_000, 15_000, 15_000);
        let connect = H(WinHttpConnect(session.0, &HSTRING::from(u.host.as_str()), u.port, 0));
        if connect.0.is_null() {
            return Err("нет связи с сервером".into());
        }
        let flags = if u.secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let req = H(WinHttpOpenRequest(connect.0, w!("GET"), &HSTRING::from(u.path.as_str()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if req.0.is_null() {
            return Err("запрос не создан".into());
        }
        let headers: Vec<u16> = format!("Accept: {}\r\n", accept).encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0).map_err(|_| "нет связи с сервером".to_string())?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|_| "сервер не ответил".to_string())?;
        let mut status = 0u32;
        let mut len = 4u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut _ as *mut c_void), &mut len, &mut index)
            .map_err(|_| "непонятный ответ".to_string())?;
        if status != 200 {
            return Err(format!("HTTP {}", status));
        }
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n).map_err(|_| "связь оборвалась".to_string())?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
            if out.len() > max {
                return Err("ответ слишком большой".into());
            }
        }
        Ok(out)
    }
}

/// SHA-256 строкой hex (маленькими буквами). None — BCrypt не сработал.
pub fn sha256(data: &[u8]) -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    st.is_ok().then(|| out.iter().map(|b| format!("{:02x}", b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = parse_url("https://api.github.com/repos/a/b/releases/latest").unwrap();
        assert_eq!((u.secure, u.host.as_str(), u.port, u.path.as_str()), (true, "api.github.com", 443, "/repos/a/b/releases/latest"));
        assert_eq!(parse_url("https://example.com").unwrap().path, "/");
        let local = parse_url("http://127.0.0.1:8123/x").unwrap();
        assert_eq!((local.secure, local.port), (false, 8123));
        // Без шифрования только до своего компьютера; логины в адресе и другие схемы не принимаем.
        assert!(parse_url("http://example.com/x").is_none());
        assert!(parse_url("https://user@evil.com/").is_none());
        assert!(parse_url("ftp://x/").is_none());
        assert!(parse_url("https://:443/").is_none());
    }

    #[test]
    fn sha() {
        assert_eq!(sha256(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
