//! Bounded Part 21 syntax reader. Geometry is resolved separately from metadata.
use crate::Result;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) enum Value {
    Number(f64),
    Ref(usize),
    List(Vec<Value>),
    Text(String),
    Enum(String),
    Typed(String, Vec<Value>),
    Empty,
}
impl Value {
    pub fn number(&self) -> Result<f64> {
        match self {
            Self::Number(n) => Ok(*n),
            Self::Typed(name, args)
                if args.len() == 1 && (name.ends_with("_MEASURE") || name == "PARAMETER_VALUE") =>
            {
                args[0].number()
            }
            Self::Text(s) => Err(crate::Error::step(format!(
                "STEP: 文字列 {s:?} を数値として使えません"
            ))),
            _ => Err(crate::Error::step("STEP: 数値が必要です".into())),
        }
    }
    pub fn integer(&self) -> Result<usize> {
        let n = self.number()?;
        if (0.0..=1_000_000.0).contains(&n) && n.fract() == 0. {
            Ok(n as usize)
        } else {
            Err(crate::Error::step("STEP: 整数の範囲が不正です".into()))
        }
    }
    pub fn reference(&self) -> Result<usize> {
        match self {
            Self::Ref(id) => Ok(*id),
            _ => Err(crate::Error::step("STEP: #番号による参照が必要です".into())),
        }
    }
    pub fn list(&self) -> Result<&[Value]> {
        match self {
            Self::List(values) => Ok(values),
            _ => Err(crate::Error::step("STEP: リストが必要です".into())),
        }
    }
    pub fn refs(&self) -> Result<Vec<usize>> {
        self.list()?.iter().map(Self::reference).collect()
    }
    pub fn numbers(&self) -> Result<Vec<f64>> {
        self.list()?.iter().map(Self::number).collect()
    }
    pub fn boolean(&self) -> Result<bool> {
        match self {
            Self::Enum(s) if s == "T" => Ok(true),
            Self::Enum(s) if s == "F" => Ok(false),
            _ => Err(crate::Error::step("STEP: .T. または .F. が必要です".into())),
        }
    }
}

#[derive(Debug)]
pub(super) struct Entity {
    pub parts: Vec<(String, Vec<Value>)>,
}
impl Entity {
    pub fn part(&self, name: &str) -> Option<&[Value]> {
        self.parts
            .iter()
            .find(|(kind, _)| kind == name)
            .map(|(_, fields)| fields.as_slice())
    }
    pub fn has(&self, name: &str) -> bool {
        self.part(name).is_some()
    }
    pub fn kind(&self) -> String {
        self.parts
            .iter()
            .map(|p| p.0.as_str())
            .collect::<Vec<_>>()
            .join("+")
    }
}

#[derive(Debug)]
pub(super) struct Table {
    pub entities: BTreeMap<usize, Entity>,
}
pub(super) fn field(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i)
        .ok_or_else(|| crate::Error::step(format!("STEP: 属性{}がありません", i + 1)))
}
impl Table {
    pub fn get(&self, id: usize) -> Result<&Entity> {
        self.entities
            .get(&id)
            .ok_or_else(|| crate::Error::step(format!("STEP: 参照 #{id} がありません")))
    }
    pub fn args(&self, id: usize, kind: &str) -> Result<&[Value]> {
        let e = self.get(id)?;
        e.part(kind).ok_or_else(|| {
            crate::Error::step(format!("STEP #{id}: {kind}が必要です（{}）", e.kind()))
        })
    }
    pub fn read(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 128 * 1024 * 1024 {
            return Err(crate::Error::step(
                "STEP: ファイルの読込上限128 MiBを超えています".into(),
            ));
        }
        let mut parser = Parser {
            bytes,
            at: 0,
            values: 0,
        };
        let mut entities = BTreeMap::new();
        if !parser.find_data()? {
            return Err(crate::Error::step(
                "STEP: Part 21のDATA節がありません".into(),
            ));
        }
        loop {
            parser.space()?;
            if parser.bytes.get(parser.at) != Some(&b'#') {
                if parser.ident()? != "ENDSEC" {
                    return parser.error("DATA節の終端が不正です");
                }
                parser.take(b';')?;
                break;
            }
            parser.at += 1;
            let id = parser.id()?;
            parser.take(b'=')?;
            let mut parts = Vec::new();
            parser.space()?;
            if parser.bytes.get(parser.at) == Some(&b'(') {
                parser.at += 1;
                loop {
                    parser.space()?;
                    if parser.bytes.get(parser.at) == Some(&b')') {
                        parser.at += 1;
                        break;
                    }
                    let name = parser.ident()?;
                    parts.push((name, parser.arguments(0)?));
                    if parts.len() > 32 {
                        return parser.error("複合エンティティの部分型が32個を超えています");
                    }
                }
            } else {
                let name = parser.ident()?;
                parts.push((name, parser.arguments(0)?));
            }
            if parts.is_empty()
                || parts
                    .iter()
                    .enumerate()
                    .any(|(i, p)| parts[..i].iter().any(|q| q.0 == p.0))
            {
                return parser.error("空または重複した複合エンティティです");
            }
            parser.take(b';')?;
            if entities.insert(id, Entity { parts }).is_some() {
                return Err(crate::Error::step(format!("STEP: #{id} が重複しています")));
            }
            if entities.len() > 250_000 {
                return parser.error("エンティティ数の上限250,000を超えています");
            }
        }
        if parser.find_data()? {
            return Err(crate::Error::step("STEP: 複数DATA節は未対応です".into()));
        }
        Ok(Self { entities })
    }

    pub fn length_scale_mm(&self) -> Result<f64> {
        self.measure_scale("LENGTH_UNIT", "METRE", "LENGTH_MEASURE_WITH_UNIT", 1000.)
    }
    pub fn angle_scale_rad(&self) -> Result<f64> {
        self.measure_scale(
            "PLANE_ANGLE_UNIT",
            "RADIAN",
            "PLANE_ANGLE_MEASURE_WITH_UNIT",
            1.,
        )
    }
    fn measure_scale(&self, kind: &str, symbol: &str, measure: &str, base: f64) -> Result<f64> {
        let mut scales = Vec::new();
        for e in self.entities.values() {
            if let Some(a) = e.part("GLOBAL_UNIT_ASSIGNED_CONTEXT") {
                for id in field(a, 0)?.refs()? {
                    if self.get(id)?.has(kind) {
                        scales.push(self.unit_scale(id, 0, symbol, measure, base)?);
                    }
                }
            }
        }
        let Some(&scale) = scales.first() else {
            return Err(crate::Error::step(format!(
                "STEP: {kind}の単位が定義されていません"
            )));
        };
        if !scale.is_finite()
            || scale <= 0.
            || scales.iter().any(|s| (s - scale).abs() > scale * 1e-12)
        {
            return Err(crate::Error::step(format!(
                "STEP: {kind}の単位が不正、または異なる単位の複数表現です"
            )));
        }
        Ok(scale)
    }
    fn unit_scale(
        &self,
        id: usize,
        depth: usize,
        symbol: &str,
        measure_kind: &str,
        base: f64,
    ) -> Result<f64> {
        if depth > 16 {
            return Err(crate::Error::step(
                "STEP: 単位の参照が循環しています".into(),
            ));
        }
        let e = self.get(id)?;
        if let Some(a) = e.part("SI_UNIT") {
            let prefix = match field(a, 0)? {
                Value::Empty => 1.,
                Value::Enum(s) => match s.as_str() {
                    "EXA" => 1e18,
                    "PETA" => 1e15,
                    "TERA" => 1e12,
                    "GIGA" => 1e9,
                    "MEGA" => 1e6,
                    "KILO" => 1e3,
                    "HECTO" => 1e2,
                    "DECA" => 10.,
                    "DECI" => 1e-1,
                    "CENTI" => 1e-2,
                    "MILLI" => 1e-3,
                    "MICRO" => 1e-6,
                    "NANO" => 1e-9,
                    "PICO" => 1e-12,
                    "FEMTO" => 1e-15,
                    "ATTO" => 1e-18,
                    _ => {
                        return Err(crate::Error::step(format!(
                            "STEP #{id}: 未対応のSI接頭辞です"
                        )));
                    }
                },
                _ => {
                    return Err(crate::Error::step(format!(
                        "STEP #{id}: SI接頭辞が不正です"
                    )));
                }
            };
            if !matches!(field(a,1)?, Value::Enum(s) if s == symbol) {
                return Err(crate::Error::step(format!(
                    "STEP #{id}: SI単位に{symbol}が必要です"
                )));
            }
            return Ok(base * prefix);
        }
        if let Some(a) = e.part("CONVERSION_BASED_UNIT") {
            let measure = self.get(field(a, 1)?.reference()?)?;
            let m = measure
                .part(measure_kind)
                .or_else(|| measure.part("MEASURE_WITH_UNIT"))
                .ok_or_else(|| {
                    crate::Error::step(format!("STEP #{id}: {measure_kind}の換算係数がありません"))
                })?;
            return Ok(field(m, 0)?.number()?
                * self.unit_scale(
                    field(m, 1)?.reference()?,
                    depth + 1,
                    symbol,
                    measure_kind,
                    base,
                )?);
        }
        Err(crate::Error::step(format!("STEP #{id}: 未対応の単位です")))
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    values: usize,
}
impl Parser<'_> {
    fn error<T>(&self, message: &str) -> Result<T> {
        Err(crate::Error::step(format!(
            "STEP byte {}: {message}",
            self.at
        )))
    }
    fn space(&mut self) -> Result<()> {
        loop {
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
            if self.bytes.get(self.at..self.at + 2) != Some(b"/*") {
                return Ok(());
            }
            self.at += 2;
            while self.bytes.get(self.at..self.at + 2) != Some(b"*/") {
                if self.at >= self.bytes.len() {
                    return self.error("コメントが閉じていません");
                }
                self.at += 1;
            }
            self.at += 2;
        }
    }
    fn take(&mut self, byte: u8) -> Result<()> {
        self.space()?;
        if self.bytes.get(self.at) != Some(&byte) {
            return self.error("区切り文字が不正です");
        }
        self.at += 1;
        Ok(())
    }
    fn ident(&mut self) -> Result<String> {
        self.space()?;
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            self.at += 1;
        }
        if start == self.at {
            return self.error("エンティティ名が必要です");
        }
        Ok(String::from_utf8_lossy(&self.bytes[start..self.at]).to_ascii_uppercase())
    }
    fn id(&mut self) -> Result<usize> {
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .ok_or_else(|| crate::Error::step(format!("STEP byte {start}: 参照番号が不正です")))
    }
    fn string(&mut self) -> Result<String> {
        self.take(b'\'')?;
        let mut bytes = Vec::new();
        loop {
            let Some(&c) = self.bytes.get(self.at) else {
                return self.error("文字列が閉じていません");
            };
            self.at += 1;
            if c == b'\'' {
                if self.bytes.get(self.at) == Some(&b'\'') {
                    self.at += 1;
                } else {
                    break;
                }
            }
            bytes.push(c);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    fn arguments(&mut self, depth: usize) -> Result<Vec<Value>> {
        if depth > 64 {
            return self.error("リストの深さが上限64を超えています");
        }
        self.take(b'(')?;
        self.space()?;
        let mut values = Vec::new();
        if self.bytes.get(self.at) == Some(&b')') {
            self.at += 1;
            return Ok(values);
        }
        loop {
            values.push(self.value(depth + 1)?);
            self.space()?;
            match self.bytes.get(self.at) {
                Some(b')') => {
                    self.at += 1;
                    return Ok(values);
                }
                Some(b',') => self.at += 1,
                _ => return self.error("リストの区切りが不正です"),
            }
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.space()?;
        self.values += 1;
        if self.values > 2_000_000 {
            return self.error("属性数の上限2,000,000を超えています");
        }
        let Some(&c) = self.bytes.get(self.at) else {
            return self.error("属性が途中で終了しています");
        };
        match c {
            b'$' | b'*' => {
                self.at += 1;
                Ok(Value::Empty)
            }
            b'#' => {
                self.at += 1;
                self.id().map(Value::Ref)
            }
            b'\'' => self.string().map(Value::Text),
            b'(' => self.arguments(depth).map(Value::List),
            b'.' if !self.bytes.get(self.at + 1).is_some_and(u8::is_ascii_digit) => {
                self.at += 1;
                let s = self.ident()?;
                self.take(b'.')?;
                Ok(Value::Enum(s))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_digit() || b"+-.eEdD".contains(c))
                {
                    self.at += 1;
                }
                let value =
                    String::from_utf8_lossy(&self.bytes[start..self.at]).replace(['d', 'D'], "E");
                let number = value
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| {
                        crate::Error::step(format!("STEP byte {start}: 有限の数値が必要です"))
                    })?;
                Ok(Value::Number(number))
            }
            _ if c.is_ascii_alphabetic() => {
                let name = self.ident()?;
                Ok(Value::Typed(name, self.arguments(depth)?))
            }
            _ => self.error("未対応の属性表記です"),
        }
    }
    fn find_data(&mut self) -> Result<bool> {
        while self.at < self.bytes.len() {
            self.space()?;
            match self.bytes.get(self.at) {
                Some(b'\'') => {
                    self.string()?;
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    if self.ident()? == "DATA" {
                        self.space()?;
                        if self.bytes.get(self.at) == Some(&b'(') {
                            self.arguments(0)?;
                        }
                        self.take(b';')?;
                        return Ok(true);
                    }
                }
                Some(_) => self.at += 1,
                None => break,
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn part21_handles_complex_entities_comments_and_quoted_separators() {
        let text = b"ISO-10303-21;HEADER;FILE_NAME('DATA;#99=bad;',());ENDSEC;DATA;/*#4*/#1=CARTESIAN_POINT('a''b;#9',(1.,-2.E-3,.5));#2=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));#3=(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#2)) REPRESENTATION_CONTEXT('',''));ENDSEC;END-ISO-10303-21;";
        let t = Table::read(text).unwrap();
        assert_eq!(t.entities.len(), 3);
        assert_eq!(
            t.args(1, "CARTESIAN_POINT").unwrap()[1].numbers().unwrap(),
            vec![1., -0.002, 0.5]
        );
        assert_eq!(t.length_scale_mm().unwrap(), 1.);
        for text in [
            "DATA;#1=A();#1=B();ENDSEC;",
            "DATA;#1=A(1e999);ENDSEC;",
            "DATA;#1=A((1,));ENDSEC;",
            "DATA;#1=A('bad);ENDSEC;",
        ] {
            assert!(Table::read(text.as_bytes()).is_err());
        }
    }
    #[test]
    fn conversion_based_angles_and_length_contexts_are_validated() {
        let text=b"DATA;#1=(PLANE_ANGLE_UNIT() NAMED_UNIT(*) SI_UNIT($,.RADIAN.));#2=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.017453292519943295),#1);#3=(CONVERSION_BASED_UNIT('DEGREE',#2) NAMED_UNIT(*) PLANE_ANGLE_UNIT());#4=GLOBAL_UNIT_ASSIGNED_CONTEXT((#3));ENDSEC;";
        let t = Table::read(text).unwrap();
        assert!((t.angle_scale_rad().unwrap() - std::f64::consts::PI / 180.).abs() < 1e-16);
        assert!(t.length_scale_mm().is_err());
        let invalid = String::from_utf8(text.to_vec()).unwrap().replace(
            "GLOBAL_UNIT_ASSIGNED_CONTEXT((#3))",
            "GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#3))",
        );
        assert!(
            Table::read(invalid.as_bytes())
                .unwrap()
                .angle_scale_rad()
                .is_err()
        );
    }
}
