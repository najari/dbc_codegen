//! Validated multiplex dependencies and interval predicates shared by all stages.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

use anyhow::{Context, Result, ensure};
use can_dbc::{Dbc, Message, MultiplexIndicator, Signal, ValueDescription, ValueType};
use quote::ToTokens;
use serde::Serialize;

use crate::{
    Config,
    preparation::ieee_type,
    signal_type::ValType,
    utils::{MessageExt, SignalExt},
};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Range {
    pub min: u64,
    pub max: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct Dependency {
    pub parent: usize,
    pub ranges: Vec<Range>,
}

pub(crate) struct Plan {
    pub selectors: Vec<bool>,
    pub parents: Vec<Option<Dependency>>,
    pub conditions: Vec<BTreeMap<usize, Vec<Range>>>,
    pub order: Vec<usize>,
}

pub(crate) fn is_selector(s: &Signal) -> bool {
    matches!(
        s.multiplexer_indicator,
        MultiplexIndicator::Multiplexor | MultiplexIndicator::MultiplexorAndMultiplexedSignal(_)
    )
}

pub(crate) fn general(dbc: &Dbc, msg: &Message) -> bool {
    dbc.extended_multiplex
        .iter()
        .any(|v| v.message_id == msg.id)
        || msg.signals.iter().filter(|s| is_selector(s)).count() > 1
        || msg.signals.iter().any(|s| {
            matches!(
                s.multiplexer_indicator,
                MultiplexIndicator::MultiplexorAndMultiplexedSignal(_)
            ) || (is_selector(s) && s.value_type == ValueType::Signed)
        })
}

fn normalize(mut ranges: Vec<Range>) -> Vec<Range> {
    ranges.sort_by_key(|r| (r.min, r.max));
    let mut result: Vec<Range> = Vec::new();
    for r in ranges {
        if let Some(last) = result.last_mut()
            && r.min <= last.max.saturating_add(1)
        {
            last.max = last.max.max(r.max);
            continue;
        }
        result.push(r);
    }
    result
}

impl Plan {
    #[allow(clippy::float_cmp)]
    pub(crate) fn build(dbc: &Dbc, msg: &Message) -> Result<Self> {
        let selectors: Vec<_> = msg.signals.iter().map(is_selector).collect();
        let mut parents: Vec<Option<Dependency>> = vec![None; msg.signals.len()];
        let index = |name: &str| {
            msg.signals
                .iter()
                .position(|s| s.name == name)
                .with_context(|| format!("mux references missing signal `{name}`"))
        };
        for mapping in dbc
            .extended_multiplex
            .iter()
            .filter(|v| v.message_id == msg.id)
        {
            let child = index(&mapping.signal_name)?;
            let parent = index(&mapping.multiplexor_signal_name)?;
            ensure!(
                selectors[parent],
                "mux parent `{}` is not a selector",
                mapping.multiplexor_signal_name
            );
            ensure!(
                child != parent,
                "self-referencing mux `{}`",
                mapping.signal_name
            );
            let dep = parents[child].get_or_insert_with(|| Dependency {
                parent,
                ranges: Vec::new(),
            });
            ensure!(
                dep.parent == parent,
                "multiple parents for mux signal `{}` are unsupported",
                mapping.signal_name
            );
            ensure!(
                !mapping.mappings.is_empty(),
                "empty mux range on `{}`",
                mapping.signal_name
            );
            for r in &mapping.mappings {
                ensure!(
                    r.min_value <= r.max_value,
                    "reversed mux range on `{}`",
                    mapping.signal_name
                );
                dep.ranges.push(Range {
                    min: r.min_value,
                    max: r.max_value,
                });
            }
        }
        let roots: Vec<_> = msg
            .signals
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                (s.multiplexer_indicator == MultiplexIndicator::Multiplexor && parents[i].is_none())
                    .then_some(i)
            })
            .collect();
        for (i, s) in msg.signals.iter().enumerate() {
            if parents[i].is_none()
                && let MultiplexIndicator::MultiplexedSignal(key)
                | MultiplexIndicator::MultiplexorAndMultiplexedSignal(key) =
                    s.multiplexer_indicator
            {
                ensure!(
                    roots.len() == 1,
                    "missing or ambiguous multiplexor for `{}`",
                    s.name
                );
                parents[i] = Some(Dependency {
                    parent: roots[0],
                    ranges: vec![Range { min: key, max: key }],
                });
            }
            if selectors[i] {
                ensure!(
                    s.factor == 1.0 && s.offset == 0.0 && ieee_type(dbc, msg, s).is_none(),
                    "selector `{}` must be an unscaled integer",
                    s.name
                );
            }
        }
        for dep in parents.iter_mut().flatten() {
            let width = msg.signals[dep.parent].size;
            ensure!(
                (1..=64).contains(&width),
                "selector wire width must be 1..=64"
            );
            let max = u64::MAX
                .checked_shr(u32::try_from(64u64.saturating_sub(width))?)
                .context("invalid selector width")?;
            ensure!(
                dep.ranges.iter().all(|r| r.max <= max),
                "mux range exceeds selector `{}` wire width",
                msg.signals[dep.parent].name
            );
            dep.ranges = normalize(std::mem::take(&mut dep.ranges));
        }
        let mut result = Self {
            selectors,
            parents,
            conditions: vec![BTreeMap::new(); msg.signals.len()],
            order: Vec::new(),
        };
        let mut state = vec![0; msg.signals.len()];
        for i in 0..msg.signals.len() {
            result.visit(i, &mut state, msg)?;
        }
        Ok(result)
    }

    fn visit(&mut self, i: usize, state: &mut [u8], msg: &Message) -> Result<()> {
        ensure!(
            state[i] != 1,
            "cyclic mux dependency at `{}`",
            msg.signals[i].name
        );
        if state[i] == 2 {
            return Ok(());
        }
        state[i] = 1;
        if let Some(dep) = self.parents[i].clone() {
            self.visit(dep.parent, state, msg)?;
            self.conditions[i] = self.conditions[dep.parent].clone();
            self.conditions[i].insert(dep.parent, dep.ranges);
        }
        state[i] = 2;
        self.order.push(i);
        Ok(())
    }

    pub(crate) fn mutually_exclusive(&self, a: usize, b: usize) -> bool {
        self.conditions[a].iter().any(|(selector, left)| {
            self.conditions[b].get(selector).is_some_and(|right| {
                !left
                    .iter()
                    .any(|x| right.iter().any(|y| x.min <= y.max && y.min <= x.max))
            })
        })
    }

    pub(crate) fn predicate(&self, msg: &Message, i: usize) -> Result<String> {
        let parts: Result<Vec<_>> = self.conditions[i]
            .iter()
            .map(|(parent, ranges)| {
                let s = &msg.signals[*parent];
                let raw = crate::read_fn_with_type(s, msg, ValType::from_signal_uint(s))?;
                let choices: Vec<_> = ranges
                    .iter()
                    .map(|r| {
                        format!(
                            "({}_u64..={}_u64).contains(&u64::from({raw}))",
                            r.min, r.max
                        )
                    })
                    .collect();
                Ok(format!("({})", choices.join(" || ")))
            })
            .collect();
        let parts = parts?;
        Ok(if parts.is_empty() {
            "true".into()
        } else {
            parts.join(" && ")
        })
    }
}

impl Config<'_> {
    pub(crate) fn render_general_signal(
        &self,
        w: &mut impl Write,
        dbc: &Dbc,
        msg: &Message,
        plan: &Plan,
        i: usize,
    ) -> Result<()> {
        let mut s = msg.signals[i].clone();
        let mut lookup = dbc.clone();
        if plan.selectors[i] {
            s.multiplexer_indicator = MultiplexIndicator::Multiplexor;
            lookup.value_descriptions.retain(|v| !matches!(v, ValueDescription::Signal {message_id,name,..} if *message_id == msg.id && *name == s.name));
        }
        let field = s.field_name();
        let active: syn::Ident = syn::parse_str(&format!("{field}_is_active"))?;
        let error: syn::Expr = syn::parse_str(&format!(
            "CanError::InactiveSignal {{ message_id: Self::MESSAGE_ID, signal: {:?} }}",
            s.name
        ))?;
        let guard: syn::Stmt = syn::parse_quote!(if !self.#active() { return Err(#error); });
        writeln!(
            w,
            "/// Whether `{}` is active, including every parent condition.",
            s.name
        )?;
        writeln!(
            w,
            "pub fn {active}(&self) -> bool {{ {} }}",
            plan.predicate(msg, i)?
        )?;
        let mut buf = Vec::new();
        self.render_signal(&mut buf, &s, &lookup, msg)?;
        let mut code = String::from_utf8(buf)?;
        let conditional = plan.parents[i].is_some();
        if conditional {
            code = code.replace(&format!("self.{field}()"), &format!("self.{field}()?"));
        }
        let mut methods: syn::ItemImpl = syn::parse_str(&format!("impl X {{ {code} }}"))?;
        for item in &mut methods.items {
            if let syn::ImplItem::Fn(method) = item {
                let name = method.sig.ident.to_string();
                if conditional && name == field {
                    let typ: syn::Type = syn::parse_str(&self.public_type(&lookup, msg, &s))?;
                    let body = method.block.clone();
                    method.sig.output = syn::parse_quote!(-> Result<#typ, CanError>);
                    method.block = syn::parse_quote!({ #guard Ok(#body) });
                } else if conditional
                    && (name == format!("set_{field}")
                        || name == format!("set_{field}_raw_val")
                        || name == format!("set_{field}_quantized"))
                {
                    method.block.stmts.insert(0, guard.clone());
                }
                if name == format!("{field}_raw_val") {
                    method.attrs.push(syn::parse_quote!(#[doc = "Reads stored wire bits even when this signal is inactive."]));
                }
                if plan.selectors[i] && name == format!("set_{field}") {
                    // Selector physical setters remain private; select_* uses wire bits.
                    method.attrs.push(syn::parse_quote!(#[allow(dead_code)]));
                }
            }
            writeln!(w, "{}", item.to_token_stream())?;
        }
        if plan.selectors[i] {
            writeln!(
                w,
                "/// Select unsigned wire bits; preserves all other bits and permits unknown branches."
            )?;
            writeln!(
                w,
                "pub fn select_{field}(&mut self, value: u64) -> Result<(), CanError> {{"
            )?;
            if conditional {
                writeln!(w, "{guard}", guard = guard.to_token_stream())?;
            }
            let max = u64::MAX
                .checked_shr(u32::try_from(64u64.saturating_sub(s.size))?)
                .context("invalid selector width")?;
            writeln!(
                w,
                "if value > {max}_u64 {{ return Err(CanError::ParameterOutOfRange {{ message_id: Self::MESSAGE_ID }}); }}"
            )?;
            s.value_type = ValueType::Unsigned;
            crate::pack_bits(w, &s, msg)?;
            writeln!(w, "Ok(()) }}")?;
        }
        Ok(())
    }

    pub(crate) fn render_general_arbitrary(
        w: &mut impl Write,
        msg: &Message,
        plan: &Plan,
    ) -> Result<()> {
        let ty = msg.type_name();
        writeln!(
            w,
            "impl<'a> Arbitrary<'a> for {ty} {{ fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self, arbitrary::Error> {{"
        )?;
        writeln!(
            w,
            "let mut res = Self::new().map_err(|_| arbitrary::Error::IncorrectFormat)?; u.fill_buffer(&mut res.raw)?;"
        )?;
        for i in plan.order.iter().copied().filter(|i| plan.selectors[*i]) {
            let field = msg.signals[i].field_name();
            let choices: BTreeSet<_> = plan
                .parents
                .iter()
                .flatten()
                .filter(|d| d.parent == i)
                .flat_map(|d| d.ranges.iter().map(|r| (r.min, r.max)))
                .collect();
            let choices: Vec<_> = if choices.is_empty() {
                vec![(0, 0)]
            } else {
                choices.into_iter().collect()
            };
            writeln!(
                w,
                "if res.{field}_is_active() {{ let bounds = *u.choose(&{choices:?})?; let value = u.int_in_range(bounds.0..=bounds.1)?; res.select_{field}(value).map_err(|_| arbitrary::Error::IncorrectFormat)?; }}"
            )?;
        }
        writeln!(w, "Ok(res) }} }}")?;
        Ok(())
    }
}
