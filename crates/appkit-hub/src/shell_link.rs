//! A Windows shortcut (`.lnk`) written and read as a file, by the format Microsoft documents
//! ([MS-SHLLINK]), without the script host or COM.
//!
//! The link written here: a header, an item list of the path (the shell takes the target from
//! it), a `LinkInfo` with the local path (ANSI and Unicode), and the strings for the description,
//! the working folder and the arguments. No icon: the shell shows that of the target, which the
//! program carries itself (rule APP-11). Reading is the other half for `uninstall`: the target
//! path of a link from its `LinkInfo`, also of one the script host made before.
//!
//! [MS-SHLLINK]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/

/// What a link starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The program, an absolute path.
    pub target: String,
    pub arguments: String,
    pub working_dir: String,
    /// The text the shell shows as the comment of the link.
    pub description: String,
    /// How the window of the program is shown (`SW_*`; 7 is minimized without focus).
    pub show_command: u32,
}

const HEADER_SIZE: u32 = 0x4C;
/// `00021401-0000-0000-C000-000000000046`, in the byte order of the file.
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
const HAS_LINK_TARGET_ID_LIST: u32 = 0x01;
const HAS_LINK_INFO: u32 = 0x02;
const HAS_NAME: u32 = 0x04;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const IS_UNICODE: u32 = 0x80;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
/// `LinkInfo` with the Unicode offsets (a header of 0x24 bytes).
const LINK_INFO_HEADER_SIZE: u32 = 0x24;
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 0x01;
const DRIVE_FIXED: u32 = 3;
/// The item of "This PC" (`{20D04FE0-3AEA-1069-A2D8-08002B30309D}`), the root of a path on a drive.
const MY_COMPUTER_ITEM: [u8; 20] = [
    0x14, 0x00, 0x1F, 0x50, 0xE0, 0x4F, 0xD0, 0x20, 0xEA, 0x3A, 0x69, 0x10, 0xA2, 0xD8, 0x08, 0x00,
    0x2B, 0x30, 0x30, 0x9D,
];
/// The size of the item of a drive (`/C:\`, filled with NUL).
const DRIVE_ITEM_SIZE: usize = 0x19;
/// An item of a folder or a file whose name is in Unicode (0x30 | 0x04, and 0x01 a folder, 0x02 a
/// file).
const FOLDER_ITEM: u8 = 0x35;
const FILE_ITEM: u8 = 0x36;
const FILE_ATTRIBUTE_DIRECTORY: u16 = 0x10;

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// The ANSI form of a path, for readers that know no Unicode: a character outside ASCII is `?`
/// (a reader of this time takes the Unicode path, which is always written too).
fn ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .collect()
}

/// The item list of a path on a drive (`C:\a\b.exe`): "This PC", the drive, then an item for
/// every folder and the file, each with its name in Unicode. The shell takes the target of a link
/// from this list (a link with only `LinkInfo` has no `TargetPath` for it). `None` for a path that
/// is not on a drive letter (a share), which gets no list.
fn id_list(target: &str) -> Option<Vec<u8>> {
    let bytes = target.as_bytes();
    let on_drive =
        bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !on_drive {
        return None;
    }
    let parts: Vec<&str> = target[3..].split('\\').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut list = Vec::new();
    list.extend_from_slice(&MY_COMPUTER_ITEM);
    let mut drive = vec![0u8; DRIVE_ITEM_SIZE];
    drive[0] = u8::try_from(DRIVE_ITEM_SIZE).ok()?;
    drive[2] = 0x2F;
    drive[3..6].copy_from_slice(&bytes[..3]);
    list.extend_from_slice(&drive);
    for (i, name) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let mut item = Vec::new();
        item.push(if last { FILE_ITEM } else { FOLDER_ITEM });
        item.push(0);
        u32le(&mut item, 0); // file size: unknown
        u32le(&mut item, 0); // date and time of the last write: unknown
        u16le(
            &mut item,
            if last {
                FILE_ATTRIBUTE_ARCHIVE as u16
            } else {
                FILE_ATTRIBUTE_DIRECTORY
            },
        );
        for unit in utf16(name) {
            u16le(&mut item, unit);
        }
        u16le(&mut item, 0);
        let size = u16::try_from(item.len() + 2).ok()?;
        u16le(&mut list, size);
        list.extend_from_slice(&item);
    }
    u16le(&mut list, 0); // the end of the list
    Some(list)
}

fn size_u32(len: usize) -> Result<u32, String> {
    u32::try_from(len).map_err(|_| "the link would be too large".to_owned())
}

/// The bytes of the `.lnk` file for `link`.
///
/// # Errors
///
/// A string is longer than a link can hold, or holds a NUL.
pub fn to_bytes(link: &Link) -> Result<Vec<u8>, String> {
    let strings = [
        link.description.as_str(),
        link.working_dir.as_str(),
        link.arguments.as_str(),
    ];
    if strings
        .iter()
        .chain([&link.target.as_str()])
        .any(|s| s.contains('\0'))
    {
        return Err("a text of the link holds a NUL".to_owned());
    }
    let list = id_list(&link.target);
    let mut out = Vec::with_capacity(512);

    // ShellLinkHeader.
    u32le(&mut out, HEADER_SIZE);
    out.extend_from_slice(&LINK_CLSID);
    let with_list = if list.is_some() {
        HAS_LINK_TARGET_ID_LIST
    } else {
        0
    };
    u32le(
        &mut out,
        with_list | HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ARGUMENTS | IS_UNICODE,
    );
    u32le(&mut out, FILE_ATTRIBUTE_ARCHIVE);
    out.extend_from_slice(&[0; 24]); // creation, access and write time: unknown
    u32le(&mut out, 0); // file size
    u32le(&mut out, 0); // icon index
    u32le(&mut out, link.show_command);
    u16le(&mut out, 0); // hot key
    out.extend_from_slice(&[0; 10]); // reserved

    // LinkTargetIDList: its size, then the list.
    if let Some(list) = &list {
        let size = u16::try_from(list.len()).map_err(|_| "the path of the program is too long")?;
        u16le(&mut out, size);
        out.extend_from_slice(list);
    }

    // LinkInfo: the header, a VolumeID of a fixed drive without a label, the path.
    let volume: Vec<u8> = {
        let mut v = Vec::new();
        u32le(&mut v, 17);
        u32le(&mut v, DRIVE_FIXED);
        u32le(&mut v, 0); // serial number: unknown
        u32le(&mut v, 0x10); // the label follows
        v.push(0);
        v
    };
    let mut base = ansi(&link.target);
    base.push(0);
    let suffix = [0u8];
    let mut base_w: Vec<u8> = Vec::new();
    for unit in utf16(&link.target) {
        u16le(&mut base_w, unit);
    }
    u16le(&mut base_w, 0);
    let suffix_w = [0u8, 0];

    let volume_at = LINK_INFO_HEADER_SIZE as usize;
    let base_at = volume_at + volume.len();
    let suffix_at = base_at + base.len();
    let base_w_at = suffix_at + suffix.len();
    let suffix_w_at = base_w_at + base_w.len();
    let total = suffix_w_at + suffix_w.len();
    u32le(&mut out, size_u32(total)?);
    u32le(&mut out, LINK_INFO_HEADER_SIZE);
    u32le(&mut out, VOLUME_ID_AND_LOCAL_BASE_PATH);
    u32le(&mut out, size_u32(volume_at)?);
    u32le(&mut out, size_u32(base_at)?);
    u32le(&mut out, 0); // no network path
    u32le(&mut out, size_u32(suffix_at)?);
    u32le(&mut out, size_u32(base_w_at)?);
    u32le(&mut out, size_u32(suffix_w_at)?);
    out.extend_from_slice(&volume);
    out.extend_from_slice(&base);
    out.extend_from_slice(&suffix);
    out.extend_from_slice(&base_w);
    out.extend_from_slice(&suffix_w);

    // StringData, in the order of the format: name, working folder, arguments.
    for text in strings {
        let units = utf16(text);
        // A string of `StringData` is counted in UTF-16 units in a `u16`.
        let count = u16::try_from(units.len()).map_err(|_| "a text of the link is too long")?;
        u16le(&mut out, count);
        for unit in units {
            u16le(&mut out, unit);
        }
    }

    // ExtraData: only the terminal block.
    u32le(&mut out, 0);
    Ok(out)
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<usize> {
    let v = u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
    usize::try_from(v).ok()
}

/// The local target path a link names, from its `LinkInfo` (the Unicode one when there is one).
/// `None` for a file that is no link or names no local path (a link to a share, or one that has
/// only an item list).
#[must_use]
pub fn target_path(bytes: &[u8]) -> Option<String> {
    if read_u32(bytes, 0)? != HEADER_SIZE as usize || bytes.get(4..20)? != LINK_CLSID {
        return None;
    }
    let flags = u32::try_from(read_u32(bytes, 0x14)?).ok()?;
    let mut at = HEADER_SIZE as usize;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        at = at.checked_add(2 + usize::from(read_u16(bytes, at)?))?;
    }
    if flags & HAS_LINK_INFO == 0 {
        return None;
    }
    let info = bytes.get(at..at.checked_add(read_u32(bytes, at)?)?)?;
    let header = read_u32(info, 4)?;
    let info_flags = read_u32(info, 8)?;
    if info_flags & VOLUME_ID_AND_LOCAL_BASE_PATH as usize == 0 {
        return None;
    }
    if header >= LINK_INFO_HEADER_SIZE as usize {
        let w_at = read_u32(info, 0x1C)?;
        if w_at != 0 {
            let units: Vec<u16> = info
                .get(w_at..)?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .take_while(|u| *u != 0)
                .collect();
            return String::from_utf16(&units).ok();
        }
    }
    let a_at = read_u32(info, 0x10)?;
    let ansi: Vec<u8> = info
        .get(a_at..)?
        .iter()
        .copied()
        .take_while(|b| *b != 0)
        .collect();
    Some(String::from_utf8_lossy(&ansi).into_owned())
}

/// Write `link` as the file `path`: beside it first, then moved over, so a link that is cut short
/// never replaces one that works.
///
/// # Errors
///
/// The link cannot be made or the folder is not writable.
pub fn write(path: &std::path::Path, link: &Link) -> Result<(), String> {
    let bytes = to_bytes(link)?;
    let mut part = path.as_os_str().to_owned();
    part.push(".new");
    let part = std::path::PathBuf::from(part);
    std::fs::write(&part, bytes).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("cannot write {}: {e}", part.display())
    })?;
    std::fs::rename(&part, path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("cannot write {}: {e}", path.display())
    })
}

/// Delete the link `path` when its target is a file named `program` (compared without case, as
/// Windows does): a link of the person's own that has the same name and another target stays.
/// Returns whether one was deleted; no file there is no error.
///
/// # Errors
///
/// A link of ours that cannot be deleted, or a file there that cannot be read.
pub fn remove_if_target(path: &std::path::Path, program: &str) -> Result<bool, String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let ours = target_path(&bytes).is_some_and(|t| {
        t.rsplit(['\\', '/'])
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case(program))
    });
    if !ours {
        return Ok(false);
    }
    std::fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    fn sample(target: &str) -> Link {
        Link {
            target: target.to_owned(),
            arguments: "start --open".to_owned(),
            working_dir: r"C:\Apps".to_owned(),
            description: "rules-desktop: start the hub and open the page".to_owned(),
            show_command: 7,
        }
    }

    #[test]
    fn a_link_names_its_target_and_reads_back() {
        let bytes = to_bytes(&sample(r"C:\Apps\rules-hub.exe")).unwrap();
        assert_eq!(&bytes[..4], &[0x4C, 0, 0, 0]);
        assert_eq!(&bytes[4..20], &LINK_CLSID);
        assert_eq!(
            target_path(&bytes).as_deref(),
            Some(r"C:\Apps\rules-hub.exe")
        );
        // The show command is where the header keeps it.
        assert_eq!(read_u32(&bytes, 0x3C), Some(7));
    }

    #[test]
    fn a_non_ascii_path_survives_in_the_unicode_path() {
        let path = r"C:\Users\Müller\AppData\Local\rules-desktop\bin\rules-hub.exe";
        let bytes = to_bytes(&sample(path)).unwrap();
        assert_eq!(target_path(&bytes).as_deref(), Some(path));
    }

    #[test]
    fn the_strings_follow_link_info_counted_in_utf16_units() {
        let link = sample(r"C:\a.exe");
        let bytes = to_bytes(&link).unwrap();
        let list_at = HEADER_SIZE as usize;
        let info_at = list_at + 2 + usize::from(read_u16(&bytes, list_at).unwrap());
        let mut at = info_at + read_u32(&bytes, info_at).unwrap();
        for text in [&link.description, &link.working_dir, &link.arguments] {
            let n = usize::from(read_u16(&bytes, at).unwrap());
            let units: Vec<u16> = bytes[at + 2..at + 2 + 2 * n]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            assert_eq!(&String::from_utf16(&units).unwrap(), text);
            at += 2 + 2 * n;
        }
        // Only the terminal block is left.
        assert_eq!(&bytes[at..], &[0, 0, 0, 0]);
    }

    #[test]
    fn an_item_list_before_link_info_is_skipped() {
        // A path that gets no list of ours (not on a drive letter), with a foreign list of 6
        // bytes put after the header, as another program writes one.
        let path = r"\\?\C:\Apps\rules-hub.exe";
        let mut bytes = to_bytes(&sample(path)).unwrap();
        let flags = u32::try_from(read_u32(&bytes, 0x14).unwrap()).unwrap();
        assert_eq!(flags & HAS_LINK_TARGET_ID_LIST, 0);
        bytes[0x14..0x18].copy_from_slice(&(flags | HAS_LINK_TARGET_ID_LIST).to_le_bytes());
        let list = [4u8, 0, 0xAA, 0xBB, 0, 0];
        bytes.splice(0x4C..0x4C, list);
        assert_eq!(target_path(&bytes).as_deref(), Some(path));
        // Ours, for a path on a drive, is skipped too.
        let ours = to_bytes(&sample(r"C:\Apps\rules-hub.exe")).unwrap();
        let flags = u32::try_from(read_u32(&ours, 0x14).unwrap()).unwrap();
        assert_ne!(flags & HAS_LINK_TARGET_ID_LIST, 0);
        assert_eq!(
            target_path(&ours).as_deref(),
            Some(r"C:\Apps\rules-hub.exe")
        );
    }

    #[test]
    fn a_file_that_is_no_link_names_no_target() {
        assert_eq!(target_path(b""), None);
        assert_eq!(target_path(&[0x4C, 0, 0, 0]), None);
        let mut bytes = to_bytes(&sample(r"C:\a.exe")).unwrap();
        bytes[4] ^= 0xFF;
        assert_eq!(target_path(&bytes), None);
        // Cut in the middle of LinkInfo.
        let whole = to_bytes(&sample(r"C:\a.exe")).unwrap();
        assert_eq!(target_path(&whole[..0x60]), None);
    }

    #[test]
    fn only_a_link_to_the_program_is_removed() {
        let dir = Scratch::new("shell-link");
        let ours = dir.join("Rules.lnk");
        write(&ours, &sample(r"C:\Apps\Rules-Hub.EXE")).unwrap();
        assert!(!dir.join("Rules.lnk.new").exists());
        let theirs = dir.join("Mine.lnk");
        write(&theirs, &sample(r"C:\Apps\editor.exe")).unwrap();
        let not_a_link = dir.join("Text.lnk");
        std::fs::write(&not_a_link, b"just text").unwrap();

        assert!(remove_if_target(&ours, "rules-hub.exe").unwrap());
        assert!(!ours.exists());
        assert!(!remove_if_target(&theirs, "rules-hub.exe").unwrap());
        assert!(theirs.exists());
        assert!(!remove_if_target(&not_a_link, "rules-hub.exe").unwrap());
        assert!(not_a_link.exists());
        // No link there at all.
        assert!(!remove_if_target(&ours, "rules-hub.exe").unwrap());
    }

    /// The shell itself reads the link back (through its COM object, from PowerShell): the
    /// format is the one Windows takes, also with a path outside ASCII.
    #[cfg(windows)]
    #[test]
    fn the_shell_reads_the_link_as_written() {
        let dir = Scratch::new("shell-com");
        let folder = dir.join("Müller äö");
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("rules-hub.exe");
        std::fs::write(&target, b"").unwrap();
        let link_path = dir.join("Rules.lnk");
        let mut link = sample(&target.to_string_lossy());
        link.working_dir = folder.to_string_lossy().into_owned();
        write(&link_path, &link).unwrap();

        let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
             $l = (New-Object -ComObject WScript.Shell).CreateShortcut($env:APPKIT_LNK); \
             [Console]::Out.Write($l.TargetPath + '|' + $l.Arguments + '|' + $l.WorkingDirectory + '|' + $l.WindowStyle + '|' + $l.Description)";
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("APPKIT_LNK", &link_path)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{text}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            text,
            format!(
                "{}|start --open|{}|7|{}",
                target.display(),
                folder.display(),
                link.description
            )
        );
    }

    /// A link names no icon of its own, so the shell shows the icon of its target: a program that
    /// carries its icon (rule APP-11) has it on its links too. The target is a program of Windows,
    /// whose icon is not the plain one of a file that carries none.
    #[cfg(windows)]
    #[test]
    fn the_shell_shows_the_icon_of_the_target_for_a_link() {
        let dir = Scratch::new("shell-icon");
        let target =
            std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("explorer.exe");
        let link_path = dir.join("Rules.lnk");
        write(&link_path, &sample(&target.to_string_lossy())).unwrap();
        let plain = dir.join("plain.exe");
        std::fs::write(&plain, b"").unwrap();

        let script = r#"
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class ShellIcon {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct INFO { public IntPtr hIcon; public int iIcon; public uint attr;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string name;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 80)] public string type; }
  [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
  static extern IntPtr SHGetFileInfo(string path, uint attrs, ref INFO info, uint size, uint flags);
  public static IntPtr Large(string path) { var i = new INFO(); SHGetFileInfo(path, 0, ref i, (uint)Marshal.SizeOf(i), 0x100); return i.hIcon; }
}
'@
foreach ($path in $env:APPKIT_LNK, $env:APPKIT_TARGET, $env:APPKIT_PLAIN) {
    $icon = [Drawing.Icon]::FromHandle([ShellIcon]::Large($path)).ToBitmap()
    # The right half: a link has the arrow of the shell at the bottom left.
    $half = $icon.Clone([Drawing.Rectangle]::new($icon.Width / 2, 0, $icon.Width / 2, $icon.Height), $icon.PixelFormat)
    $stream = [IO.MemoryStream]::new()
    $half.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
    [Convert]::ToBase64String($stream.ToArray())
}
"#;
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            // With the value of PowerShell 7 this one loads the modules of that one.
            .env_remove("PSModulePath")
            .env("APPKIT_LNK", &link_path)
            .env("APPKIT_TARGET", &target)
            .env("APPKIT_PLAIN", &plain)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let icons: Vec<&str> = text.lines().collect();
        assert_eq!(icons.len(), 3, "{text}");
        assert_eq!(icons[0], icons[1], "the link shows another icon");
        assert_ne!(icons[1], icons[2], "the target shows the plain icon");
    }

    #[test]
    fn a_nul_in_a_text_is_refused() {
        let mut link = sample(r"C:\a.exe");
        link.arguments = "a\0b".to_owned();
        assert!(to_bytes(&link).is_err());
    }
}
