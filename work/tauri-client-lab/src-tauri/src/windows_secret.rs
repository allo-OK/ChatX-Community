use std::{io, ptr::{null, null_mut}};
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
}};

pub fn protect(input: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    let size = u32::try_from(input.len()).map_err(io::Error::other)?;
    let mut data = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let source = CRYPT_INTEGER_BLOB { cbData: size, pbData: input.as_ptr().cast_mut() };
    // Windows allocates the result and binds it to the current user profile.
    unsafe {
        let status = if encrypt {
            CryptProtectData(&source, null(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut data)
        } else {
            CryptUnprotectData(&source, null_mut(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut data)
        };
        if status == 0 { return Err(io::Error::last_os_error()); }
        if data.pbData.is_null() { return Err(io::Error::other("Windows tidak mengembalikan data")); }
        let output = std::slice::from_raw_parts_mut(data.pbData, data.cbData as usize);
        let result = output.to_vec();
        if !encrypt { output.fill(0); }
        LocalFree(data.pbData.cast());
        Ok(result)
    }
}
