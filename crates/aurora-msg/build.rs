//! Generates typed message structs and codecs from `message_template.msg`.

use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Debug)]
enum VarType {
    Simple(&'static str, &'static str), // (rust type, codec suffix)
    Fixed(usize),
    Variable(u8),
}

#[derive(Debug)]
struct Var {
    name: String,
    ty: VarType,
}

#[derive(Debug)]
enum BlockKind {
    Single,
    Multiple(usize),
    Variable,
}

#[derive(Debug)]
struct Block {
    name: String,
    kind: BlockKind,
    vars: Vec<Var>,
}

#[derive(Debug)]
struct Message {
    name: String,
    freq: String,
    number: u32,
    trusted: bool,
    zerocoded: bool,
    deprecated: bool,
    blocks: Vec<Block>,
}

fn tokenize(src: &str) -> Vec<String> {
    let mut toks = Vec::new();
    for line in src.lines() {
        let line = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        let spaced = line.replace('{', " { ").replace('}', " } ");
        toks.extend(spaced.split_whitespace().map(|s| s.to_owned()));
    }
    toks
}

fn parse_num(s: &str) -> u32 {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).expect("hex number")
    } else {
        s.parse().expect("number")
    }
}

fn parse(src: &str) -> Vec<Message> {
    let toks = tokenize(src);
    let mut i = 0;
    let mut msgs = Vec::new();
    // skip "version 2.0"
    while i < toks.len() && toks[i] != "{" {
        i += 1;
    }
    while i < toks.len() {
        assert_eq!(toks[i], "{", "expected message start at token {i}");
        i += 1;
        let name = toks[i].clone();
        let freq = toks[i + 1].clone();
        let number = parse_num(&toks[i + 2]);
        let trusted = toks[i + 3] == "Trusted";
        let zerocoded = toks[i + 4] == "Zerocoded";
        i += 5;
        let mut deprecated = false;
        while toks[i] != "{" && toks[i] != "}" {
            if toks[i] == "Deprecated" {
                deprecated = true;
            }
            i += 1;
        }
        let mut blocks = Vec::new();
        while toks[i] == "{" {
            i += 1;
            let bname = toks[i].clone();
            let kind = match toks[i + 1].as_str() {
                "Single" => {
                    i += 2;
                    BlockKind::Single
                }
                "Multiple" => {
                    let n = parse_num(&toks[i + 2]) as usize;
                    i += 3;
                    BlockKind::Multiple(n)
                }
                "Variable" => {
                    i += 2;
                    BlockKind::Variable
                }
                other => panic!("bad block kind {other}"),
            };
            let mut vars = Vec::new();
            while toks[i] == "{" {
                let vname = toks[i + 1].clone();
                let ty = match toks[i + 2].as_str() {
                    "U8" => VarType::Simple("u8", "u8"),
                    "U16" => VarType::Simple("u16", "u16"),
                    "U32" => VarType::Simple("u32", "u32"),
                    "U64" => VarType::Simple("u64", "u64"),
                    "S8" => VarType::Simple("i8", "i8"),
                    "S16" => VarType::Simple("i16", "i16"),
                    "S32" => VarType::Simple("i32", "i32"),
                    "S64" => VarType::Simple("i64", "i64"),
                    "F32" => VarType::Simple("f32", "f32"),
                    "F64" => VarType::Simple("f64", "f64"),
                    "LLVector3" => VarType::Simple("glam::Vec3", "vec3"),
                    "LLVector3d" => VarType::Simple("glam::DVec3", "dvec3"),
                    "LLVector4" => VarType::Simple("glam::Vec4", "vec4"),
                    "LLQuaternion" => VarType::Simple("glam::Quat", "quat"),
                    "LLUUID" => VarType::Simple("uuid::Uuid", "uuid"),
                    "BOOL" => VarType::Simple("bool", "bool"),
                    "IPADDR" => VarType::Simple("[u8; 4]", "ipaddr"),
                    "IPPORT" => VarType::Simple("u16", "ipport"),
                    "Fixed" => VarType::Fixed(parse_num(&toks[i + 3]) as usize),
                    "Variable" => VarType::Variable(parse_num(&toks[i + 3]) as u8),
                    other => panic!("bad var type {other}"),
                };
                i += match ty {
                    VarType::Fixed(_) | VarType::Variable(_) => 5,
                    VarType::Simple(..) => 4,
                };
                vars.push(Var { name: vname, ty });
            }
            assert_eq!(toks[i], "}", "expected block end for {bname}");
            i += 1;
            blocks.push(Block { name: bname, kind, vars });
        }
        assert_eq!(toks[i], "}", "expected message end for {name}");
        i += 1;
        msgs.push(Message {
            name,
            freq,
            number,
            trusted,
            zerocoded,
            deprecated,
            blocks,
        });
    }
    msgs
}

fn snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (idx, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev = if idx > 0 { Some(chars[idx - 1]) } else { None };
            let next = chars.get(idx + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    const KW: &[&str] = &[
        "type", "ref", "move", "loop", "match", "mod", "override", "self", "super", "use", "where", "as", "fn", "impl", "in", "for", "if",
        "else", "let", "struct", "enum", "trait", "static", "const", "box", "final", "virtual", "priv", "macro", "async", "await", "dyn",
        "abstract", "become", "do", "try", "yield", "crate", "extern", "pub", "return", "true", "false", "unsafe", "while", "break",
        "continue", "gen",
    ];
    if KW.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

fn generate(msgs: &[Message]) -> String {
    let mut s = String::new();
    s.push_str("// @generated from message_template.msg by build.rs. Do not edit.\n\n");

    for m in msgs {
        let modname = snake(&m.name);
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            "Low" => format!("MsgId::Low({})", m.number),
            "Fixed" => format!("MsgId::Low({})", m.number & 0xFFFF),
            f => panic!("bad freq {f}"),
        };

        // block structs
        let _ = writeln!(s, "pub mod {modname} {{");
        let _ = writeln!(s, "    #[allow(unused_imports)] use crate::{{Reader, Writer, DecodeError}};");
        for b in &m.blocks {
            let _ = writeln!(s, "    #[derive(Debug, Clone, Default, PartialEq)]");
            let _ = writeln!(s, "    pub struct {} {{", b.name);
            for v in &b.vars {
                let ty = match &v.ty {
                    VarType::Simple(t, _) => (*t).to_owned(),
                    VarType::Fixed(_) | VarType::Variable(_) => "Vec<u8>".to_owned(),
                };
                let _ = writeln!(s, "        pub {}: {},", snake(&v.name), ty);
            }
            let _ = writeln!(s, "    }}");
            let _ = writeln!(s, "    impl {} {{", b.name);
            let _ = writeln!(s, "        #[inline] pub fn encode(&self, w: &mut Writer) {{");
            for v in &b.vars {
                let f = snake(&v.name);
                match &v.ty {
                    VarType::Simple(_, c) => {
                        let _ = writeln!(s, "            w.{c}(self.{f});");
                    }
                    VarType::Fixed(n) => {
                        let _ = writeln!(s, "            w.fixed(&self.{f}, {n});");
                    }
                    VarType::Variable(1) => {
                        let _ = writeln!(s, "            w.var1(&self.{f});");
                    }
                    VarType::Variable(_) => {
                        let _ = writeln!(s, "            w.var2(&self.{f});");
                    }
                }
            }
            let _ = writeln!(s, "        }}");
            let _ = writeln!(s, "        #[inline] pub fn decode(r: &mut Reader) -> Result<Self, DecodeError> {{");
            let _ = writeln!(s, "            Ok(Self {{");
            for v in &b.vars {
                let f = snake(&v.name);
                match &v.ty {
                    VarType::Simple(_, c) => {
                        let _ = writeln!(s, "                {f}: r.{c}()?,");
                    }
                    VarType::Fixed(n) => {
                        let _ = writeln!(s, "                {f}: r.fixed({n})?,");
                    }
                    VarType::Variable(1) => {
                        let _ = writeln!(s, "                {f}: r.var1()?,");
                    }
                    VarType::Variable(_) => {
                        let _ = writeln!(s, "                {f}: r.var2()?,");
                    }
                }
            }
            let _ = writeln!(s, "            }})");
            let _ = writeln!(s, "        }}");
            let _ = writeln!(s, "    }}");
        }
        let _ = writeln!(s, "}}");

        // message struct
        let _ = writeln!(s, "#[derive(Debug, Clone, Default, PartialEq)]");
        let _ = writeln!(s, "pub struct {} {{", m.name);
        for b in &m.blocks {
            let f = snake(&b.name);
            match b.kind {
                BlockKind::Single => {
                    let _ = writeln!(s, "    pub {f}: {modname}::{},", b.name);
                }
                _ => {
                    let _ = writeln!(s, "    pub {f}: Vec<{modname}::{}>,", b.name);
                }
            }
        }
        let _ = writeln!(s, "}}");
        let _ = writeln!(s, "impl crate::Msg for {} {{", m.name);
        let _ = writeln!(s, "    const NAME: &'static str = \"{}\";", m.name);
        let _ = writeln!(s, "    const ID: MsgId = {id};");
        let _ = writeln!(s, "    const ZEROCODED: bool = {};", m.zerocoded);
        let _ = writeln!(s, "    const TRUSTED: bool = {};", m.trusted);
        let _ = writeln!(s, "    fn encode_body(&self, w: &mut Writer) {{");
        for b in &m.blocks {
            let f = snake(&b.name);
            match b.kind {
                BlockKind::Single => {
                    let _ = writeln!(s, "        self.{f}.encode(w);");
                }
                BlockKind::Multiple(n) => {
                    let _ = writeln!(
                        s,
                        "        for i in 0..{n} {{ match self.{f}.get(i) {{ Some(b) => b.encode(w), None => {modname}::{}::default().encode(w) }} }}",
                        b.name
                    );
                }
                BlockKind::Variable => {
                    let _ = writeln!(
                        s,
                        "        let n = self.{f}.len().min(255); w.u8(n as u8); for b in &self.{f}[..n] {{ b.encode(w); }}"
                    );
                }
            }
        }
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "    fn decode_body(r: &mut Reader) -> Result<Self, DecodeError> {{");
        let _ = writeln!(s, "        Ok(Self {{");
        for b in &m.blocks {
            let f = snake(&b.name);
            match b.kind {
                BlockKind::Single => {
                    let _ = writeln!(s, "            {f}: {modname}::{}::decode(r)?,", b.name);
                }
                BlockKind::Multiple(n) => {
                    let _ = writeln!(
                        s,
                        "            {f}: {{ let mut v = Vec::with_capacity({n}); for _ in 0..{n} {{ v.push({modname}::{}::decode(r)?); }} v }},",
                        b.name
                    );
                }
                BlockKind::Variable => {
                    let _ = writeln!(
                        s,
                        "            {f}: {{ let n = r.block_count()?; let mut v = Vec::with_capacity(n); for _ in 0..n {{ v.push({modname}::{}::decode(r)?); }} v }},",
                        b.name
                    );
                }
            }
        }
        let _ = writeln!(s, "        }})");
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "}}");
    }

    // id -> info table
    s.push_str("/// Look up static information about a message id.\n");
    s.push_str("pub fn message_info(id: MsgId) -> Option<MsgInfo> {\n    match id {\n");
    for m in msgs {
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            _ => format!("MsgId::Low({})", m.number & 0xFFFF),
        };
        let _ = writeln!(
            s,
            "        {id} => Some(MsgInfo {{ name: \"{}\", zerocoded: {}, trusted: {}, deprecated: {} }}),",
            m.name, m.zerocoded, m.trusted, m.deprecated
        );
    }
    s.push_str("        _ => None,\n    }\n}\n");
    s
}

fn main() {
    let path = "message_template.msg";
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-changed=build.rs");
    let src = std::fs::read_to_string(path).expect("read template");
    let msgs = parse(&src);
    let code = generate(&msgs);
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("messages.rs");
    std::fs::write(out, code).expect("write generated code");
}
