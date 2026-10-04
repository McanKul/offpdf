//! Operand count and types for every interpreted operator (SPEC §B.6 arity table).

use super::{malformed, LexError, Op, Operand, O};

/// Checks operand count and types for every interpreted operator.
pub fn check_arity(op: &Op) -> Result<(), LexError> {
    let a = &op.operands;
    let num = |i: usize| a.get(i).is_some_and(|o| o.as_number().is_some());
    let name = |i: usize| a.get(i).is_some_and(|o| o.as_name().is_some());
    let string = |i: usize| a.get(i).is_some_and(|o| o.as_str_bytes().is_some());
    let nums = |n: usize| a.len() == n && (0..n).all(num);
    let ok = match op.operator {
        O::b
        | O::B
        | O::bStar
        | O::BStar
        | O::BT
        | O::ET
        | O::EMC
        | O::BX
        | O::EX
        | O::f
        | O::F
        | O::fStar
        | O::h
        | O::n
        | O::q
        | O::Q
        | O::s
        | O::S
        | O::TStar
        | O::W
        | O::WStar
        | O::BI => a.is_empty(),
        O::BDC | O::DP => {
            a.len() == 2
                && name(0)
                && matches!(a.get(1), Some(Operand::Dict { .. } | Operand::Name { .. }))
        }
        O::BMC | O::MP | O::CS | O::cs | O::Do | O::gs | O::ri | O::sh => a.len() == 1 && name(0),
        O::c | O::cm | O::d1 | O::Tm => nums(6),
        O::d => {
            a.len() == 2
                && matches!(a.first(), Some(Operand::Array { items, .. }) if items.iter().all(|i| i.as_number().is_some()))
                && num(1)
        }
        O::d0 | O::l | O::m | O::Td | O::TD => nums(2),
        O::G
        | O::g
        | O::i
        | O::j
        | O::J
        | O::M
        | O::w
        | O::Tc
        | O::Tw
        | O::Tz
        | O::TL
        | O::Tr
        | O::Ts => nums(1),
        O::K | O::k | O::re | O::v | O::y => nums(4),
        O::RG | O::rg => nums(3),
        O::SC | O::sc => (1..=32).contains(&a.len()) && (0..a.len()).all(num),
        O::SCN | O::scn => {
            let n = if a.last().is_some_and(|o| o.as_name().is_some()) {
                a.len() - 1
            } else {
                a.len()
            };
            !a.is_empty() && n <= 32 && (0..n).all(num)
        }
        O::Tf => a.len() == 2 && name(0) && num(1),
        O::Tj | O::Quote => a.len() == 1 && string(0),
        O::TJ => matches!(a.as_slice(), [Operand::Array { items, .. }]
            if items.iter().all(|i| matches!(i, Operand::Str { .. } | Operand::Number { .. }))),
        O::DoubleQuote => a.len() == 3 && num(0) && num(1) && string(2),
        O::ID | O::EI => false,
        O::Unknown => true,
    };
    if ok {
        Ok(())
    } else {
        malformed(op.op_span.start, "operand mismatch")
    }
}
