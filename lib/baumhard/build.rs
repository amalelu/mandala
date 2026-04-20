use lazy_static::lazy_static;
use path_slash::PathExt;
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::{env, error::Error, fs::File, io::Write, path::Path};

const NUM_SPECIAL_FONT: usize = 1;
const ANY_STR: &'static str = "Any";
const DOC_ANY_STR: &'static str = "Indicates that the defining party does not give two fucks about the font used";

const FONT_DIR: &'static str = "src/font/fonts";
const FONTS_RS_FILE: &'static str = "generated_fonts_data.rs";
const VALID_EXTENSIONS: [&str; 2] = ["otf", "ttf"];
const MAX_LEN: usize = 20;
const MIN_LEN: usize = 5;
const MIN_TOKEN_LEN: usize = 4;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR not found");
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").expect("MANIFEST_DIR not found");
    prepare_font_file(out_dir.clone(), manifest_dir.clone())?;

    Ok(())
}

fn prepare_font_file(out_dir: OsString, manifest_dir: OsString) -> Result<(), Box<dyn Error>> {
    let manifest_path = Path::new(&manifest_dir);
    let mut fonts = collect_fonts(FONT_DIR, manifest_path)?;
    fonts.push((ANY_STR.to_string(), "".to_string()));

    let out_file = Path::new(&out_dir).join(FONTS_RS_FILE);
    generate_font_file(out_file, &fonts)
}

fn generate_font_file(
    out_file: impl AsRef<Path>,
    fonts: &[(String, String)],
) -> Result<(), Box<dyn Error>> {
    let mut file = File::create(&out_file)?;

    writeln!(file, "#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]")?;
    writeln!(file, "pub enum AppFont {{")?;
    for (name, _) in fonts {
        if name.eq(ANY_STR) {
            writeln!(file, "///{}", DOC_ANY_STR)?;
            writeln!(file, "{},", name)?;
        } else {
            writeln!(file, "{},", name)?;
        }
    }
    writeln!(file, "}}")?;
    writeln!(
        file,
        "pub(crate) static FONT_DATA: [(AppFont, &'static [u8]); {}] = [",
        fonts.len() - NUM_SPECIAL_FONT
    )?;
    for (name, path) in fonts {
        if !name.eq(ANY_STR) {
            writeln!(file, "    ({}, include_bytes!(\"{}\")),", name, path)?;
        }
    }
    writeln!(file, "];")?;
    Ok(())
}

use std::borrow::Borrow;

fn collect_fonts(
    source_dir: &str,
    manifest_dir: &Path,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut fonts_map: HashMap<String, (String, String)> = HashMap::new();
    for entry in walkdir::WalkDir::new(source_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_type().is_dir())
    {
        let path = entry.path();

        if let Some(extension) = path.extension() {
            if extension != "otf" && extension != "ttf" {
                continue;
            }
        } else {
            continue;
        }

        let filename: String = path
            .strip_prefix(source_dir)?
            .file_name()
            .expect("no file name")
            .to_str()
            .expect("no str")
            .to_string();

        let full_path: String = Cow::to_string(&manifest_dir.to_slash_lossy())
            + "/"
            + Cow::borrow(&path.to_slash_lossy());
        let filename_without_extension = filename.trim_end_matches(".otf").trim_end_matches(".ttf");
        let font_file_name = filename_without_extension
            .split_once("-")
            .unwrap_or((filename_without_extension, ""))
            .0
            .to_lowercase()
            .to_string();

        let font_name = get_font_name(&full_path)?;
        let sanitized_name = camel_case(&font_name);
        let font_camel_case;
        if sanitized_name.is_err() {
            font_camel_case = fallback_sanitize(&filename);
        } else {
            font_camel_case = sanitized_name.unwrap();
        }

        if let Some((_font, existing_path)) = fonts_map.get(&font_file_name) {
            if existing_path.ends_with(VALID_EXTENSIONS[0])
                && filename.ends_with(VALID_EXTENSIONS[1])
            {
                fonts_map.insert(
                    font_file_name.clone(),
                    (font_camel_case.clone(), full_path.clone()),
                );
            }
        } else {
            fonts_map.insert(
                font_file_name.clone(),
                (font_camel_case.clone(), full_path.clone()),
            );
        }
        println!("cargo:rerun-if-changed={}", full_path);
    }

    let fonts: Vec<(String, String)> = fonts_map.into_iter().map(|(_k, v)| v).collect();

    Ok(fonts)
}

fn fallback_sanitize(font_name: &str) -> String {
    // Remove leading and trailing numbers
    let trim_name = font_name
        .trim_start_matches(|c: char| c.is_numeric())
        .trim_end_matches(|c: char| c.is_numeric());

    // Split on any whitespace, hyphen, or underscore.
    let mut words: Vec<String> = trim_name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|s| s.len() >= MIN_TOKEN_LEN) // Only keep segments with MIN_TOKEN_LEN or more letters.
        .map(String::from)
        .collect();

    // If the input is less than or equal to MIN_LEN, return it as is
    if trim_name.len() <= MIN_LEN {
        return trim_name.to_string();
    }

    // Check first token
    if let Some(first_word) = words.get(0) {
        let first_word = first_word.clone(); // Clone first_word
        if first_word.len() >= MIN_LEN {
            return first_word;
        }

        // If the first token + second token is of length MIN_LEN or longer
        if words.get(1).map_or(0, |s| s.len()) + first_word.len() >= MIN_LEN {
            words.remove(1); // Consume the second token
            return first_word + "_" + words.get(0).unwrap_or(&"".to_string());
        }
    }

    let mut final_name = String::new();

    for word in &words {
        if final_name.len() + word.len() <= MAX_LEN {
            if !final_name.is_empty() {
                final_name.push_str("_");
            }
            final_name.push_str(word);
        } else if final_name.is_empty() {
            final_name = word.clone();
            final_name.truncate(MAX_LEN);
            break;
        } else {
            break;
        }
    }

    final_name
}

use ttf_parser::Face;

fn get_font_name(font_path: &str) -> Result<String, &'static str> {
    // Read the font data
    let font_data = std::fs::read(font_path).map_err(|_| "Could not read font file")?;

    // Create a Face instance
    let face = Face::parse(&font_data, 0).map_err(|_| "Could not parse font file")?;

    // Get the font name
    let names = face.names();

    for name in names {
        if name.name_id == ttf_parser::name_id::FULL_NAME {
            return Ok(std::str::from_utf8(name.name)
                .expect("Not UTF-8")
                .chars()
                .filter(|&c| !c.is_whitespace())
                .filter(|&c| c.is_ascii_alphanumeric())
                .collect::<String>());
        }
    }

    Err("Could not find font name")
}

fn camel_case(input: &str) -> Result<String, Box<dyn Error>> {
    lazy_static! {
        static ref RE_NUMBERS: Regex = Regex::new(r"^\d+").unwrap();
        static ref RE_WHITESPACE_ONLY: Regex = Regex::new(r"^\s*$").unwrap();
        static ref RE_NUMBERS_ONLY: Regex = Regex::new(r"^\d+$").unwrap();
    }

    if RE_WHITESPACE_ONLY.is_match(input) || RE_NUMBERS_ONLY.is_match(input) {
        return Err("invalid name".into());
    }

    let result: String;

    if let Some(m) = RE_NUMBERS.find(input) {
        result = format!("{}{}", &input[m.end()..], &input[..m.end()]);
    } else {
        result = input.into();
    }

    let result = result
        .chars()
        .filter(|&c| !",'.!?()&\"+:;".contains(c))
        .collect::<String>();

    let result = result
        .split_whitespace()
        .map(|word| {
            let mut c = word.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<String>>()
        .join("");

    Ok(result)
}
