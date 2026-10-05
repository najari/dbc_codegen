use crate::utils::{MessageExt, SignalExt};
use crate::{
    Config, FeatureConfig, PadAdapter, RoundingPolicy, ValType, is_unscaled, pack_bits,
    preparation::ieee_type, render_raw_accessors, signal_pub_type,
};
use anyhow::Result;
use can_dbc::{Dbc, Message, MultiplexIndicator, Signal, ValueType};
use std::io::Write;

pub(crate) fn render_wire_check(
    w: &mut impl Write,
    s: &Signal,
    msg: &Message,
    value: &str,
) -> Result<()> {
    let (low, high) = wire_bounds(s);
    let ty = msg.type_name();
    writeln!(
        w,
        "if ({value}) < {low}_i128 || ({value}) > {high}_i128 {{ return Err(CanError::ParameterOutOfRange {{ message_id: {ty}::MESSAGE_ID }}); }}"
    )?;
    Ok(())
}

// Preparation restricts the wire width to 1..=64, so all operations fit in i128.
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn wire_bounds(s: &Signal) -> (i128, i128) {
    if s.value_type == ValueType::Signed {
        (-(1i128 << (s.size - 1)), (1i128 << (s.size - 1)) - 1)
    } else {
        (0, (1i128 << s.size) - 1)
    }
}

pub(crate) fn integer_physical_bounds(s: &Signal) -> Result<(i128, i128)> {
    let convert = |text: String, lower: bool| -> Result<i128> {
        if let Ok(n) = text.parse::<i128>() {
            return Ok(n);
        }
        let f = text.parse::<f64>()?;
        let n = if lower { f.ceil() } else { f.floor() };
        anyhow::ensure!(
            n >= i128::MIN as f64 && n < i128::MAX as f64,
            "integer physical bound exceeds i128"
        );
        Ok(n as i128)
    };
    Ok((
        convert(s.min.to_string(), true)?,
        convert(s.max.to_string(), false)?,
    ))
}

impl Config<'_> {
    pub(crate) fn float_type(&self, dbc: &Dbc, msg: &Message, s: &Signal) -> Option<&'static str> {
        if let Some(typ) = ieee_type(dbc, msg, s) {
            return Some(typ);
        }
        if s.size == 1
            || s.multiplexer_indicator == MultiplexIndicator::Multiplexor
            || dbc.value_descriptions_for_signal(msg.id, &s.name).is_some()
        {
            return None;
        }
        if self.physical_f64 {
            return Some("f64");
        }
        match ValType::from_signal(s) {
            ValType::F32 => Some("f32"),
            ValType::F64 => Some("f64"),
            _ => None,
        }
    }

    pub(crate) fn physical_type(&self, dbc: &Dbc, msg: &Message, s: &Signal) -> String {
        self.float_type(dbc, msg, s)
            .map_or_else(|| ValType::from_signal(s).to_string(), str::to_owned)
    }

    pub(crate) fn public_type(&self, dbc: &Dbc, msg: &Message, s: &Signal) -> String {
        self.float_type(dbc, msg, s)
            .map_or_else(|| signal_pub_type(dbc, msg, s), str::to_owned)
    }

    pub(crate) fn render_float_signal(
        &self,
        w: &mut impl Write,
        s: &Signal,
        dbc: &Dbc,
        msg: &Message,
        typ: &str,
    ) -> Result<()> {
        let field = s.field_name();
        let ieee = ieee_type(dbc, msg, s);
        writeln!(w, "/// Physical value of original signal {:?}.", s.name)?;
        writeln!(w, "pub fn {field}(&self) -> {typ} {{")?;
        if let Some(wire) = ieee {
            if is_unscaled(s) {
                writeln!(w, "{wire}::from_bits(self.{field}_raw_val())")?;
            } else {
                writeln!(
                    w,
                    "(({wire}::from_bits(self.{field}_raw_val()) as f64) * {}_f64 + {}_f64) as {typ}",
                    s.factor, s.offset
                )?;
            }
        } else {
            writeln!(
                w,
                "((self.{field}_raw_val() as f64) * {}_f64 + {}_f64) as {typ}",
                s.factor, s.offset
            )?;
        }
        writeln!(w, "}}")?;
        let mut raw = s.clone();
        if ieee.is_some() {
            raw.value_type = ValueType::Unsigned;
        }
        render_raw_accessors(w, &raw, msg)?;
        self.render_float_setter(w, s, dbc, msg, typ)
    }

    pub(crate) fn render_float_setter(
        &self,
        w: &mut impl Write,
        s: &Signal,
        dbc: &Dbc,
        msg: &Message,
        typ: &str,
    ) -> Result<()> {
        let field = s.field_name();
        let ty = msg.type_name();
        let error = format!("CanError::ParameterOutOfRange {{ message_id: {ty}::MESSAGE_ID }}");
        writeln!(
            w,
            "/// Sets a finite physical value. Failure leaves the payload unchanged."
        )?;
        writeln!(
            w,
            "pub fn set_{field}(&mut self, value: {typ}) -> Result<(), CanError> {{ self.set_{field}_quantized(value).map(|_| ()) }}"
        )?;
        writeln!(
            w,
            "/// Sets a value and returns its actual quantized physical value."
        )?;
        writeln!(
            w,
            "pub fn set_{field}_quantized(&mut self, value: {typ}) -> Result<{typ}, CanError> {{"
        )?;
        let mut w = PadAdapter::wrap(w);
        writeln!(w, "if !value.is_finite() {{ return Err({error}); }}")?;
        if !matches!(self.check_ranges, FeatureConfig::Never) {
            if let FeatureConfig::Gated(g) = self.check_ranges {
                writeln!(w, "#[cfg(feature = {g:?})]")?;
            }
            writeln!(
                w,
                "if (value as f64) < {}_f64 || (value as f64) > {}_f64 {{ return Err({error}); }}",
                s.min, s.max
            )?;
        }
        if let Some(wire) = ieee_type(dbc, msg, s) {
            if is_unscaled(s) {
                writeln!(w, "let actual = value;")?;
                writeln!(w, "let value = value.to_bits();")?;
            } else {
                writeln!(
                    w,
                    "let wire_value = ((value as f64 - {}_f64) / {}_f64) as {wire};",
                    s.offset, s.factor
                )?;
                writeln!(w, "if !wire_value.is_finite() {{ return Err({error}); }}")?;
                writeln!(
                    w,
                    "let actual = (wire_value as f64 * {}_f64 + {}_f64) as {typ};",
                    s.factor, s.offset
                )?;
                writeln!(w, "let value = wire_value.to_bits();")?;
            }
            self.render_actual_float_check(&mut w, s, &error)?;
            let mut raw = s.clone();
            raw.value_type = ValueType::Unsigned;
            pack_bits(&mut w, &raw, msg)?;
        } else {
            let (low, high) = wire_bounds(s);
            writeln!(
                w,
                "let scaled = (value as f64 - {}_f64) / {}_f64;",
                s.offset, s.factor
            )?;
            writeln!(w, "if !scaled.is_finite() {{ return Err({error}); }}")?;
            let upper = high.saturating_add(1);
            // Avoid std-only float rounding methods in generated embedded code.
            writeln!(
                w,
                "if scaled < ({low}_f64 - 1.0) || scaled > ({upper}_f64 + 1.0) {{ return Err({error}); }}"
            )?;
            writeln!(w, "let base = scaled as i128;")?;
            let round = match self.rounding {
                RoundingPolicy::Truncate | RoundingPolicy::Exact => "base",
                RoundingPolicy::NearestAway => {
                    "base + if scaled - base as f64 >= 0.5 { 1 } else if scaled - base as f64 <= -0.5 { -1 } else { 0 }"
                }
            };
            if matches!(self.rounding, RoundingPolicy::Exact) {
                writeln!(w, "if scaled != base as f64 {{ return Err({error}); }}")?;
            }
            writeln!(w, "let bits = {round};")?;
            render_wire_check(&mut w, s, msg, "bits")?;
            let raw_type = ValType::from_signal_int(s);
            writeln!(w, "let value = bits as {raw_type};")?;
            writeln!(
                w,
                "let actual = ((value as f64) * {}_f64 + {}_f64) as {typ};",
                s.factor, s.offset
            )?;
            self.render_actual_float_check(&mut w, s, &error)?;
            pack_bits(&mut w, s, msg)?;
        }
        writeln!(w, "Ok(actual)")?;
        writeln!(w, "}}")?;
        Ok(())
    }

    fn render_actual_float_check(&self, w: &mut impl Write, s: &Signal, error: &str) -> Result<()> {
        writeln!(w, "if !actual.is_finite() {{ return Err({error}); }}")?;
        if !matches!(self.check_ranges, FeatureConfig::Never) {
            if let FeatureConfig::Gated(g) = self.check_ranges {
                writeln!(w, "#[cfg(feature = {g:?})]")?;
            }
            writeln!(
                w,
                "if (actual as f64) < {}_f64 || (actual as f64) > {}_f64 {{ return Err({error}); }}",
                s.min, s.max
            )?;
        }
        Ok(())
    }
}
