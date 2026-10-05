use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use can_dbc::{
    AttributeValueForRelationType, ByteOrder, Comment, Dbc, Message, MessageId, MultiplexIndicator,
    Signal, SignalExtendedValueType, ValueDescription, ValueType,
};
use can_dbc_pest::{DbcParser, Parser, Rule};
use serde::Serialize;

use crate::utils::{
    MessageExt, SignalExt, enum_name, is_valid_ident, multiplex_enum_name,
    multiplexed_enum_variant_name,
};
use crate::{Config, be_start_end_bit, le_start_end_bit, message_ignored};

/// Rounding applied before converting physical values to integer wire values.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundingPolicy {
    /// Truncate toward zero.
    #[default]
    Truncate,
    /// Round to nearest, with ties away from zero.
    NearestAway,
    /// Reject a physical value that cannot be represented exactly.
    Exact,
}

#[derive(Serialize)]
pub(crate) struct SignalMapping {
    pub source_name: String,
    pub field_name: String,
    pub enum_name: Option<String>,
    pub start_bit: u64,
    pub bit_width: u64,
    pub byte_order: String,
    pub receivers: Vec<String>,
    pub wire_type: String,
    pub enum_variants: Vec<serde_json::Value>,
    pub reserved_names: Vec<String>,
    pub mux: serde_json::Value,
}

#[derive(Serialize)]
pub(crate) struct MessageMapping {
    pub ordinal: usize,
    pub source_name: String,
    pub type_name: String,
    pub id: u32,
    pub extended: bool,
    pub size: u64,
    pub transmitters: Vec<String>,
    pub signals: Vec<SignalMapping>,
    pub generated_types: Vec<String>,
    pub mux_api: &'static str,
}

pub(crate) fn ieee_type(dbc: &Dbc, msg: &Message, signal: &Signal) -> Option<&'static str> {
    dbc.signal_extended_value_type_list.iter().find_map(|v| {
        if v.message_id != msg.id || v.signal_name != signal.name {
            return None;
        }
        match v.signal_extended_value_type {
            SignalExtendedValueType::IEEEfloat32Bit => Some("f32"),
            SignalExtendedValueType::IEEEdouble64bit => Some("f64"),
            SignalExtendedValueType::SignedOrUnsignedInteger => None,
        }
    })
}

// DBC value descriptions sometimes express signed fault values as unsigned wire bits.
#[allow(clippy::arithmetic_side_effects)] // Width is validated to 1..=64 before calling.
pub(crate) fn canonical_variant_value(value: i64, signal: &Signal) -> i64 {
    if signal.value_type == ValueType::Signed
        && signal.size < 64
        && value >= 0
        && i128::from(value) >= 1i128 << (signal.size - 1)
    {
        (i128::from(value) - (1i128 << signal.size)) as i64
    } else {
        value
    }
}

pub(crate) fn validate_source_ids(content: &str) -> Result<()> {
    // Check before can-dbc narrows standard IDs to u16 and masks extended bits.
    fn independent(pair: &can_dbc_pest::Pair<'_, Rule>) -> bool {
        pair.clone().into_inner().any(|p| {
            p.as_rule() == Rule::message_name && p.as_str() == "VECTOR__INDEPENDENT_SIG_MSG"
        })
    }
    fn visit(pair: can_dbc_pest::Pair<'_, Rule>, mut allow_independent: bool) -> Result<()> {
        if pair.as_rule() == Rule::message {
            allow_independent = independent(&pair);
        }
        if pair.as_rule() == Rule::message_id {
            let raw: u64 = pair.as_str().parse().context("invalid source message ID")?;
            // Vector's special independent-signal pseudo message is not a CAN frame.
            if raw != 0xc000_0000 || !allow_independent {
                ensure!(
                    raw <= 0xffff_ffff && (raw <= 0x7ff || (raw & 0xe000_0000 == 0x8000_0000)),
                    "invalid source CAN ID {raw} at line {}: standard ID must be <= 0x7ff; extended ID requires bit 31 and only bits 0..28",
                    pair.line_col().0
                );
            }
        }
        for child in pair.into_inner() {
            visit(child, allow_independent)?;
        }
        Ok(())
    }
    let pairs = DbcParser::parse(Rule::file, content)?;
    let has_independent = pairs
        .clone()
        .any(|p| p.as_rule() == Rule::message && independent(&p));
    for pair in pairs {
        visit(pair, has_independent)?;
    }
    Ok(())
}

// Bit arithmetic is bounded by the checked <=64-byte payload and <=64-bit widths.
#[allow(clippy::arithmetic_side_effects, clippy::float_cmp)]
fn validate(dbc: &Dbc) -> Result<()> {
    for v in &dbc.extended_multiplex {
        ensure!(
            dbc.messages.iter().any(|m| m.id == v.message_id),
            "mux references missing message {:?}",
            v.message_id
        );
    }
    let mut ids = BTreeSet::new();
    for msg in dbc.messages.iter().filter(|m| !message_ignored(m)) {
        let context = format!("message `{}` ({:?})", msg.name, msg.id);
        (|| -> Result<()> {
            let (extended, id) = match msg.id {
                MessageId::Standard(id) => (false, u32::from(id)),
                MessageId::Extended(id) => (true, id),
            };
            ensure!(
                id <= if extended { 0x1fff_ffff } else { 0x7ff },
                "invalid CAN ID range/extended marker"
            );
            ensure!(
                ids.insert((extended, id)),
                "duplicate CAN identity; ambiguous dispatch is unsupported"
            );
            ensure!(
                matches!(msg.size, 0..=8 | 12 | 16 | 20 | 24 | 32 | 48 | 64),
                "invalid CAN/CAN FD payload length {}",
                msg.size
            );
            for s in &msg.signals {
                ensure!(
                    (1..=64).contains(&s.size),
                    "signal `{}` wire width must be 1..=64, got {}",
                    s.name,
                    s.size
                );
            }
            let plan = crate::mux::Plan::build(dbc, msg)?;
            let mut names = BTreeSet::new();
            for s in &msg.signals {
                (|| -> Result<()> {
                    ensure!(names.insert(&s.name), "duplicate source signal name");
                    ensure!(
                        (1..=64).contains(&s.size),
                        "wire width must be 1..=64, got {}",
                        s.size
                    );
                    ensure!(
                        s.factor.is_finite() && s.factor != 0.0 && s.offset.is_finite(),
                        "factor must be finite/nonzero and offset finite"
                    );
                    let min = s.min.to_string().parse::<f64>()?;
                    let max = s.max.to_string().parse::<f64>()?;
                    ensure!(
                        min.is_finite() && max.is_finite() && min <= max,
                        "invalid physical min/max"
                    );
                    match s.byte_order {
                        ByteOrder::LittleEndian => {
                            le_start_end_bit(s, msg)?;
                        }
                        ByteOrder::BigEndian => {
                            be_start_end_bit(s, msg)?;
                        }
                    }
                    if let Some(typ) = ieee_type(dbc, msg, s) {
                        ensure!(
                            s.size == if typ == "f32" { 32 } else { 64 },
                            "{typ} declaration has wrong wire width"
                        );
                        ensure!(
                            dbc.value_descriptions_for_signal(msg.id, &s.name).is_none(),
                            "IEEE value-description enums are unsupported"
                        );
                    }
                    Ok(())
                })()
                .with_context(|| format!("signal `{}`", s.name))?;
            }
            // Simultaneously active fields may not overlap. Different mux branches may.
            for (i, a) in msg.signals.iter().enumerate() {
                for (j, b) in msg.signals.iter().enumerate().skip(i + 1) {
                    if plan.mutually_exclusive(i, j) {
                        continue;
                    }
                    let bits = |s: &Signal| -> BTreeSet<u64> {
                        match s.byte_order {
                            ByteOrder::LittleEndian => {
                                (s.start_bit..s.start_bit + s.size).collect()
                            }
                            ByteOrder::BigEndian => {
                                let start = s.start_bit / 8 * 8 + 7 - s.start_bit % 8;
                                (start..start + s.size)
                                    .map(|p| p / 8 * 8 + 7 - p % 8)
                                    .collect()
                            }
                        }
                    };
                    ensure!(
                        bits(a).is_disjoint(&bits(b)),
                        "signals `{}` and `{}` overlap while active",
                        a.name,
                        b.name
                    );
                }
            }
            Ok(())
        })()
        .with_context(|| context)?;
    }
    let mut types = BTreeSet::new();
    for v in &dbc.signal_extended_value_type_list {
        ensure!(
            dbc.signal_by_name(v.message_id, &v.signal_name).is_some(),
            "SIG_VALTYPE_ references missing signal `{}` ({:?})",
            v.signal_name,
            v.message_id
        );
        ensure!(
            types.insert((format!("{:?}", v.message_id), &v.signal_name)),
            "duplicate SIG_VALTYPE_ for `{}`",
            v.signal_name
        );
    }
    for v in &dbc.value_descriptions {
        if let ValueDescription::Signal {
            message_id,
            name,
            value_descriptions,
        } = v
        {
            let s = dbc.signal_by_name(*message_id, name).with_context(|| {
                format!("VAL_ references missing signal `{name}` ({message_id:?})")
            })?;
            if dbc
                .messages
                .iter()
                .any(|m| m.id == *message_id && message_ignored(m))
            {
                continue;
            }
            let mut values = BTreeSet::new();
            for value in value_descriptions {
                ensure!(
                    values.insert(canonical_variant_value(value.id, s)),
                    "duplicate/aliased VAL_ key on `{name}`"
                );
                let raw = i128::from(value.id);
                let (lo, hi) = if s.value_type == ValueType::Signed {
                    (-(1i128 << (s.size - 1)), (1i128 << s.size) - 1)
                } else {
                    (0, (1i128 << s.size) - 1)
                };
                ensure!(
                    (lo..=hi).contains(&raw),
                    "VAL_ key outside wire range on `{name}`"
                );
            }
        }
    }
    let message_exists = |id: MessageId| dbc.messages.iter().any(|m| m.id == id);
    let require_signal = |id: MessageId, name: &str| -> Result<()> {
        ensure!(
            dbc.signal_by_name(id, name).is_some(),
            "metadata references missing signal `{name}` ({id:?})"
        );
        Ok(())
    };
    for v in &dbc.message_transmitters {
        ensure!(
            message_exists(v.message_id),
            "BO_TX_BU_ references missing message {:?}",
            v.message_id
        );
    }
    for v in &dbc.attribute_values_message {
        ensure!(
            message_exists(v.message_id),
            "attribute `{}` references missing message {:?}",
            v.name,
            v.message_id
        );
    }
    for v in &dbc.attribute_values_signal {
        require_signal(v.message_id, &v.signal_name)?;
    }
    for v in &dbc.signal_groups {
        ensure!(
            message_exists(v.message_id),
            "SIG_GROUP_ references missing message {:?}",
            v.message_id
        );
        for name in &v.signal_names {
            require_signal(v.message_id, name)?;
        }
    }
    for v in &dbc.signal_type_refs {
        require_signal(v.message_id, &v.signal_name)?;
    }
    for v in &dbc.relation_attribute_values {
        match &v.details {
            AttributeValueForRelationType::NodeToSignal {
                message_id,
                signal_name,
                ..
            } => require_signal(*message_id, signal_name)?,
            AttributeValueForRelationType::NodeToMessage { message_id, .. } => ensure!(
                message_exists(*message_id),
                "relation attribute references missing message {message_id:?}"
            ),
        }
    }
    Ok(())
}

fn signal_names(s: &Signal) -> Vec<String> {
    let field = s.field_name();
    vec![
        field.clone(),
        format!("set_{field}"),
        format!("{field}_raw_val"),
        format!("set_{field}_raw_val"),
        format!("{field}_phys_val"),
        format!("{field}_multiplexed"),
        format!("{field}_is_active"),
        format!("select_{field}"),
        format!("set_{field}_quantized"),
        format!("{}_MIN", field.to_uppercase()),
        format!("{}_MAX", field.to_uppercase()),
    ]
}

fn family_names(dbc: &Dbc, msg: &Message) -> Result<Vec<String>> {
    let mut names = vec![msg.type_name()];
    for s in &msg.signals {
        if dbc.value_descriptions_for_signal(msg.id, &s.name).is_some() {
            names.push(enum_name(msg, s));
        }
        if s.multiplexer_indicator == MultiplexIndicator::Multiplexor
            && !crate::mux::general(dbc, msg)
        {
            names.push(multiplex_enum_name(msg, s)?);
            for branch in &msg.signals {
                if let MultiplexIndicator::MultiplexedSignal(index) = branch.multiplexer_indicator {
                    let n = multiplexed_enum_variant_name(msg, s, index)?;
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
            }
        }
    }
    Ok(names)
}

pub(crate) fn prepare(source: &Dbc, config: &Config<'_>) -> Result<(Dbc, Vec<MessageMapping>)> {
    validate(source)?;
    for n in config.selected_nodes {
        ensure!(
            source.nodes.iter().any(|node| node.0 == *n),
            "selected node `{n}` is not declared"
        );
    }
    let mut dbc = source.clone();
    let mut registry: BTreeSet<String> = [
        "Messages",
        "CanError",
        "Id",
        "StandardId",
        "ExtendedId",
        "Result",
        "Option",
        "Some",
        "None",
        "Ok",
        "Err",
        "BitArray",
        "LocalBits",
        "Lsb0",
        "Msb0",
        "BitOr",
        "Self",
        "DBC_FILE_NAME",
        "DBC_FILE_VERSION",
        "Arbitrary",
        "Unstructured",
        "UnstructuredFloatExt",
        "Serialize",
        "Deserialize",
    ]
    .map(String::from)
    .into();
    let mut remap = BTreeMap::new();
    let mut manifest = Vec::new();
    let mut selected = BTreeSet::new();
    for (ordinal, original) in source
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| !message_ignored(m))
    {
        let mut transmitters: Vec<_> = original.transmitter.iter().cloned().collect();
        for extra in source
            .message_transmitters
            .iter()
            .filter(|v| v.message_id == original.id)
        {
            for n in &extra.transmitter {
                if !transmitters.contains(n) {
                    transmitters.push(n.clone());
                }
            }
        }
        let include = config.selected_nodes.is_empty()
            || config.selected_nodes.iter().any(|n| {
                transmitters.iter().any(|t| t == n)
                    || original
                        .signals
                        .iter()
                        .any(|s| s.receivers.iter().any(|r| r == n))
            });
        if include {
            selected.insert(format!("{:?}", original.id));
        }
        let msg = &mut dbc.messages[ordinal];
        let mux_plan = crate::mux::Plan::build(source, original)?;
        let mut methods: BTreeSet<String> = [
            "new",
            "raw",
            "MESSAGE_ID",
            "MESSAGE_SIZE",
            "MESSAGE_CYCLE_TIME_MS",
        ]
        .map(String::from)
        .into();
        for s in &msg.signals {
            if let MultiplexIndicator::MultiplexedSignal(index) = s.multiplexer_indicator {
                methods.insert(format!("set_m{index}"));
            }
        }
        for (i, s) in msg.signals.iter_mut().enumerate() {
            let original_name = s.name.clone();
            let base = s.field_name();
            let mut attempt = 0usize;
            loop {
                let names = signal_names(s);
                if names
                    .iter()
                    .all(|n| is_valid_ident(n) && !methods.contains(n))
                {
                    methods.extend(names);
                    break;
                }
                attempt = attempt.checked_add(1).context("signal naming exhausted")?;
                s.name = format!("{base}_dbc_{}_{attempt}", i.saturating_add(1));
            }
            remap.insert((format!("{:?}", msg.id), original_name), s.name.clone());
        }
        // Lookups still use the original metadata at this point, so build a local remapped copy.
        let mut lookup = source.clone();
        rename_references(&mut lookup, &remap);
        let base = msg.type_name();
        let mut attempt = 0usize;
        loop {
            let family = family_names(&lookup, msg)?;
            let unique: BTreeSet<_> = family.iter().collect();
            ensure!(
                unique.len() == family.len(),
                "message `{}` has conflicting helper types",
                original.name
            );
            if family
                .iter()
                .all(|n| is_valid_ident(n) && !registry.contains(n))
            {
                registry.extend(family);
                break;
            }
            attempt = attempt.checked_add(1).context("message naming exhausted")?;
            msg.name = format!("{base}Dbc{}N{attempt}", ordinal.saturating_add(1));
        }
        if !include {
            continue;
        }
        let signals = original
            .signals
            .iter()
            .zip(&msg.signals)
            .enumerate()
            .map(|(i, (old, new))| SignalMapping {
                source_name: old.name.clone(),
                field_name: new.field_name(),
                enum_name: source
                    .value_descriptions_for_signal(original.id, &old.name)
                    .map(|_| enum_name(msg, new)),
                start_bit: old.start_bit,
                bit_width: old.size,
                byte_order: format!("{:?}", old.byte_order),
                receivers: old.receivers.clone(),
                wire_type: ieee_type(source, original, old)
                    .map_or_else(|| format!("{:?}", old.value_type), str::to_owned),
                enum_variants: source.value_descriptions_for_signal(original.id, &old.name).map_or_else(Vec::new, |variants| {
                    variants.iter().zip(crate::generate_variant_info(variants, old)).map(|(original, generated)| serde_json::json!({ "source_label": original.description, "raw_value": original.id, "canonical_raw_value": generated.value, "variant_name": generated.base_name })).collect()
                }),
                reserved_names: signal_names(new),
                mux: serde_json::json!({"selector": mux_plan.selectors[i], "parent": mux_plan.parents[i].as_ref().map(|d| serde_json::json!({"source_name": original.signals[d.parent].name, "field_name": msg.signals[d.parent].field_name(), "ranges": d.ranges}))}),
            })
            .collect();
        let (extended, id) = match original.id {
            MessageId::Standard(id) => (false, u32::from(id)),
            MessageId::Extended(id) => (true, id),
        };
        manifest.push(MessageMapping {
            ordinal,
            source_name: original.name.clone(),
            type_name: msg.type_name(),
            id,
            extended,
            size: original.size,
            transmitters,
            signals,
            generated_types: family_names(&lookup, msg)?,
            mux_api: if crate::mux::general(source, original) {
                "guarded"
            } else {
                "legacy"
            },
        });
    }
    rename_references(&mut dbc, &remap);
    dbc.messages
        .retain(|m| selected.contains(&format!("{:?}", m.id)));
    dbc.value_descriptions.retain(|v| match v {
        ValueDescription::Signal { message_id, .. } => {
            selected.contains(&format!("{message_id:?}"))
        }
        ValueDescription::EnvironmentVariable { .. } => true,
    });
    Ok((dbc, manifest))
}

fn rename_references(dbc: &mut Dbc, mapping: &BTreeMap<(String, String), String>) {
    let rename = |id: MessageId, name: &mut String| {
        if let Some(n) = mapping.get(&(format!("{id:?}"), name.clone())) {
            name.clone_from(n);
        }
    };
    for v in &mut dbc.value_descriptions {
        if let ValueDescription::Signal {
            message_id, name, ..
        } = v
        {
            rename(*message_id, name);
        }
    }
    for v in &mut dbc.extended_multiplex {
        rename(v.message_id, &mut v.signal_name);
        rename(v.message_id, &mut v.multiplexor_signal_name);
    }
    for v in &mut dbc.comments {
        if let Comment::Signal {
            message_id, name, ..
        } = v
        {
            rename(*message_id, name);
        }
    }
    for v in &mut dbc.attribute_values_signal {
        rename(v.message_id, &mut v.signal_name);
    }
    for v in &mut dbc.signal_extended_value_type_list {
        rename(v.message_id, &mut v.signal_name);
    }
    for v in &mut dbc.relation_attribute_values {
        if let AttributeValueForRelationType::NodeToSignal {
            message_id,
            signal_name,
            ..
        } = &mut v.details
        {
            rename(*message_id, signal_name);
        }
    }
}
