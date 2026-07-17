//! Dés narratifs Star Wars FFG — faces officielles, rng injectable.

use serde_json::{json, Value};

#[derive(Debug, Default, Clone, Copy)]
pub struct FfgPool {
    pub ability: u32,
    pub proficiency: u32,
    pub difficulty: u32,
    pub challenge: u32,
    pub boost: u32,
    pub setback: u32,
    pub force: u32,
}

impl FfgPool {
    pub fn from_args(args: &Value) -> Self {
        let g = |k: &str| args.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        Self {
            ability: g("ability"),
            proficiency: g("proficiency"),
            difficulty: g("difficulty"),
            challenge: g("challenge"),
            boost: g("boost"),
            setback: g("setback"),
            force: g("force"),
        }
    }
}

/// (s, f, a, t, T, D, l, d) par face.
type Face = (u32, u32, u32, u32, u32, u32, u32, u32);

const BOOST: [Face; 6] = [
    (0, 0, 0, 0, 0, 0, 0, 0),
    (0, 0, 0, 0, 0, 0, 0, 0),
    (1, 0, 0, 0, 0, 0, 0, 0),
    (1, 0, 1, 0, 0, 0, 0, 0),
    (0, 0, 2, 0, 0, 0, 0, 0),
    (0, 0, 1, 0, 0, 0, 0, 0),
];
const SETBACK: [Face; 6] = [
    (0, 0, 0, 0, 0, 0, 0, 0),
    (0, 0, 0, 0, 0, 0, 0, 0),
    (0, 1, 0, 0, 0, 0, 0, 0),
    (0, 1, 0, 0, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
];
const ABILITY: [Face; 8] = [
    (0, 0, 0, 0, 0, 0, 0, 0),
    (1, 0, 0, 0, 0, 0, 0, 0),
    (1, 0, 0, 0, 0, 0, 0, 0),
    (2, 0, 0, 0, 0, 0, 0, 0),
    (0, 0, 1, 0, 0, 0, 0, 0),
    (0, 0, 1, 0, 0, 0, 0, 0),
    (1, 0, 1, 0, 0, 0, 0, 0),
    (0, 0, 2, 0, 0, 0, 0, 0),
];
const DIFFICULTY: [Face; 8] = [
    (0, 0, 0, 0, 0, 0, 0, 0),
    (0, 1, 0, 0, 0, 0, 0, 0),
    (0, 2, 0, 0, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
    (0, 0, 0, 2, 0, 0, 0, 0),
    (0, 1, 0, 1, 0, 0, 0, 0),
];
const PROFICIENCY: [Face; 12] = [
    (0, 0, 0, 0, 0, 0, 0, 0),
    (1, 0, 0, 0, 0, 0, 0, 0),
    (1, 0, 0, 0, 0, 0, 0, 0),
    (2, 0, 0, 0, 0, 0, 0, 0),
    (2, 0, 0, 0, 0, 0, 0, 0),
    (0, 0, 1, 0, 0, 0, 0, 0),
    (1, 0, 1, 0, 0, 0, 0, 0),
    (1, 0, 1, 0, 0, 0, 0, 0),
    (1, 0, 1, 0, 0, 0, 0, 0),
    (0, 0, 2, 0, 0, 0, 0, 0),
    (0, 0, 2, 0, 0, 0, 0, 0),
    (0, 0, 0, 0, 1, 0, 0, 0),
];
const CHALLENGE: [Face; 12] = [
    (0, 0, 0, 0, 0, 0, 0, 0),
    (0, 1, 0, 0, 0, 0, 0, 0),
    (0, 1, 0, 0, 0, 0, 0, 0),
    (0, 2, 0, 0, 0, 0, 0, 0),
    (0, 2, 0, 0, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
    (0, 0, 0, 1, 0, 0, 0, 0),
    (0, 1, 0, 1, 0, 0, 0, 0),
    (0, 1, 0, 1, 0, 0, 0, 0),
    (0, 0, 0, 2, 0, 0, 0, 0),
    (0, 0, 0, 2, 0, 0, 0, 0),
    (0, 0, 0, 0, 0, 1, 0, 0),
];
const FORCE: [Face; 12] = [
    (0, 0, 0, 0, 0, 0, 0, 1),
    (0, 0, 0, 0, 0, 0, 0, 1),
    (0, 0, 0, 0, 0, 0, 0, 1),
    (0, 0, 0, 0, 0, 0, 0, 1),
    (0, 0, 0, 0, 0, 0, 0, 1),
    (0, 0, 0, 0, 0, 0, 0, 1),
    (0, 0, 0, 0, 0, 0, 0, 2),
    (0, 0, 0, 0, 0, 0, 1, 0),
    (0, 0, 0, 0, 0, 0, 1, 0),
    (0, 0, 0, 0, 0, 0, 2, 0),
    (0, 0, 0, 0, 0, 0, 2, 0),
    (0, 0, 0, 0, 0, 0, 2, 0),
];

#[derive(Debug, Default)]
pub struct FfgResult {
    pub successes: i64,
    pub failures: i64,
    pub advantages: i64,
    pub threats: i64,
    pub triumphs: i64,
    pub despairs: i64,
    pub light: i64,
    pub dark: i64,
    pub net_successes: i64,
    pub net_advantages: i64,
    pub is_success: bool,
}

pub fn roll_ffg_pool<R: FnMut() -> f64>(pool: &FfgPool, mut rng: R) -> FfgResult {
    let mut tally = (0i64, 0i64, 0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
    let mut roll = |faces: &[Face], count: u32| {
        for _ in 0..count {
            let f = faces[(rng() * faces.len() as f64) as usize % faces.len()];
            tally.0 += f.0 as i64;
            tally.1 += f.1 as i64;
            tally.2 += f.2 as i64;
            tally.3 += f.3 as i64;
            tally.4 += f.4 as i64;
            tally.5 += f.5 as i64;
            tally.6 += f.6 as i64;
            tally.7 += f.7 as i64;
        }
    };
    roll(&BOOST, pool.boost);
    roll(&SETBACK, pool.setback);
    roll(&ABILITY, pool.ability);
    roll(&DIFFICULTY, pool.difficulty);
    roll(&PROFICIENCY, pool.proficiency);
    roll(&CHALLENGE, pool.challenge);
    roll(&FORCE, pool.force);

    // Triomphe = aussi un succès ; désespoir = aussi un échec (règle officielle).
    let successes = tally.0 + tally.4;
    let failures = tally.1 + tally.5;
    let net_successes = successes - failures;
    FfgResult {
        successes,
        failures,
        advantages: tally.2,
        threats: tally.3,
        triumphs: tally.4,
        despairs: tally.5,
        light: tally.6,
        dark: tally.7,
        net_successes,
        net_advantages: tally.2 - tally.3,
        is_success: net_successes > 0,
    }
}

pub fn format_pool(pool: &FfgPool) -> String {
    let rep = |s: &str, n: u32| s.repeat(n as usize);
    let pos = format!(
        "{}{}{}{}",
        rep("🟩", pool.ability),
        rep("🟨", pool.proficiency),
        rep("🟦", pool.boost),
        rep("⬜", pool.force)
    );
    let neg = format!(
        "{}{}{}",
        rep("🟪", pool.difficulty),
        rep("🟥", pool.challenge),
        rep("⬛", pool.setback)
    );
    match (pos.is_empty(), neg.is_empty()) {
        (true, true) => "∅".into(),
        (false, true) => pos,
        (true, false) => format!("∅ vs {neg}"),
        (false, false) => format!("{pos} vs {neg}"),
    }
}

pub fn format_result(r: &FfgResult) -> String {
    let mut parts = vec![if r.net_successes > 0 {
        format!(
            "✅ Réussite ({} succès net{})",
            r.net_successes,
            if r.net_successes > 1 { "s" } else { "" }
        )
    } else {
        format!("❌ Échec ({} échec(s) net(s))", -r.net_successes)
    }];
    if r.net_advantages > 0 {
        parts.push(format!("{} avantage(s)", r.net_advantages));
    }
    if r.net_advantages < 0 {
        parts.push(format!("{} menace(s)", -r.net_advantages));
    }
    if r.triumphs > 0 {
        parts.push(format!("{} TRIOMPHE(S) ⭐", r.triumphs));
    }
    if r.despairs > 0 {
        parts.push(format!("{} DÉSESPOIR(S) 💀", r.despairs));
    }
    if r.light > 0 {
        parts.push(format!("{} lumière ○", r.light));
    }
    if r.dark > 0 {
        parts.push(format!("{} obscur ●", r.dark));
    }
    parts.join(" · ")
}

pub fn result_json(r: &FfgResult) -> Value {
    json!({
        "successes": r.successes, "failures": r.failures,
        "advantages": r.advantages, "threats": r.threats,
        "triumphs": r.triumphs, "despairs": r.despairs,
        "light": r.light, "dark": r.dark,
        "netSuccesses": r.net_successes, "netAdvantages": r.net_advantages,
        "isSuccess": r.is_success,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(vals: &[f64]) -> impl FnMut() -> f64 + '_ {
        let mut i = 0;
        move || {
            let v = vals[i % vals.len()];
            i += 1;
            v
        }
    }

    #[test]
    fn faces_deterministes() {
        // ability face 3 (double succès)
        let r = roll_ffg_pool(
            &FfgPool {
                ability: 1,
                ..Default::default()
            },
            seq(&[3.0 / 8.0]),
        );
        assert_eq!(r.net_successes, 2);
        // proficiency face 11 = triomphe (compte aussi comme succès)
        let r = roll_ffg_pool(
            &FfgPool {
                proficiency: 1,
                ..Default::default()
            },
            seq(&[11.0 / 12.0]),
        );
        assert_eq!((r.triumphs, r.successes, r.is_success), (1, 1, true));
        // challenge face 11 = désespoir
        let r = roll_ffg_pool(
            &FfgPool {
                challenge: 1,
                ..Default::default()
            },
            seq(&[11.0 / 12.0]),
        );
        assert_eq!((r.despairs, r.failures), (1, 1));
        // force : face 0 obscur, face 11 double lumière
        let r = roll_ffg_pool(
            &FfgPool {
                force: 2,
                ..Default::default()
            },
            seq(&[0.0, 11.0 / 12.0]),
        );
        assert_eq!((r.dark, r.light), (1, 2));
        // égalité = échec
        let r = roll_ffg_pool(
            &FfgPool {
                ability: 1,
                difficulty: 1,
                ..Default::default()
            },
            seq(&[3.0 / 8.0, 2.0 / 8.0]),
        );
        assert_eq!((r.net_successes, r.is_success), (0, false));
    }

    #[test]
    fn stats_plausibles() {
        let mut rng = rand::rng();
        let mut wins = 0;
        for _ in 0..10_000 {
            let r = roll_ffg_pool(
                &FfgPool {
                    ability: 2,
                    difficulty: 1,
                    ..Default::default()
                },
                || rand::Rng::random::<f64>(&mut rng),
            );
            if r.is_success {
                wins += 1;
            }
        }
        assert!((5500..8000).contains(&wins), "taux {wins}/10000 hors plage");
    }

    #[test]
    fn formats() {
        assert_eq!(
            format_pool(&FfgPool {
                ability: 2,
                proficiency: 1,
                difficulty: 2,
                ..Default::default()
            }),
            "🟩🟩🟨 vs 🟪🟪"
        );
    }
}
