//! Transistor mismatch for Xyce: the PDK's Xyce models have no mismatch sections for the
//! transistors, CDePT makes them.
//!
//! IHP's ngspice models draw the per-device mismatch in the device subcircuits:
//! `sg13_lv_nmos` of `sg13g2_moslv_mod_mismatch.lib` takes `mm_ok` and gives the
//! transistor `w = agauss(w, dw_mm, mm_ok==1)`, `l = agauss(l, dl_mm, ..)`,
//! `delvto = agauss(0, delvto_mm/sqrt(m*l*w*1e12), ..)` and
//! `factuo = agauss(1, factuo_mm/sqrt(m*l*w*1e12), ..)`. The Xyce files of the PDK
//! (`sg13g2_moslv_mod.lib`, `sg13g2_moshv_mod.lib`) have the same subcircuits without
//! them, and no `*_mm` parameters; only the MOS capacitor has its Xyce mismatch file
//! (`sg13g2_moscap_mod_mismatch.lib`), written with `if(mm_ok == 1, agauss(...), ...)`.
//!
//! Xyce can draw per instance: under `.SAMPLING useExpr=true` every `agauss()` in an
//! expression is its own random variable, also inside a subcircuit, so each instance of
//! `sg13_lv_nmos` draws for itself (the seed is `-randseed`). So the Xyce models need
//! nothing but the same expressions, and [`overlay`] makes them: a folder with the PDK's
//! Xyce files (links) in which the two MOS files are replaced by the same subcircuits
//! with `mm_ok` (default 0: the nominal device, as before) and the mismatch parameters
//! the ngspice mismatch files define. The corner files stay the PDK's; a run reads the
//! models from the folder ([`redirect`]). The numbers of sigmas are the ngspice ones,
//! so the statistics agree; the draws themselves are Xyce's own.

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail};

/// The model files made again: (Xyce file, ngspice file with the mismatch parameters).
pub const MOS_FILES: &[(&str, &str)] = &[
    ("sg13g2_moslv_mod.lib", "sg13g2_moslv_mismatch.lib"),
    ("sg13g2_moshv_mod.lib", "sg13g2_moshv_mismatch.lib"),
];

/// A subcircuit name of a MOS device that gets mismatch: `sg13_lv_nmos`, `sg13_hv_pmos`.
fn is_mos(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "sg13_lv_nmos",
        "sg13_lv_pmos",
        "sg13_hv_nmos",
        "sg13_hv_pmos",
    ]
    .contains(&n.as_str())
}

fn is_card(line: &str, card: &str) -> bool {
    line.split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case(card))
}

/// The `.param` lines of an ngspice mismatch file (the sigmas `sg13g2_lv_nmos_dw_mm`, ...),
/// without `SWSOA`, which the Xyce files do not use.
pub fn mismatch_params(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| is_card(l, ".param") && !l.to_ascii_lowercase().contains("swsoa"))
        .map(str::to_string)
        .collect()
}

/// A Xyce MOS model file with the mismatch of every `sg13_{lv,hv}_{n,p}mos` subcircuit:
/// `mm_ok=0` joins its parameters and the `YPSP103_VA` instance's `w`, `l`, `delvto` and
/// `factuo` read `agauss()`. Returns the new text and the number of subcircuits changed.
pub fn patch_mos(text: &str, params: &[String]) -> Result<(String, usize)> {
    let mut out: Vec<String> = vec![];
    let mut changed = 0usize;
    let mut sub: Option<String> = None; // the subcircuit being changed
    let mut need_params = false;
    let mut prefix = String::new();
    let mut seen_y = false;
    for line in text.lines() {
        let t = line.trim();
        if sub.is_none() {
            if is_card(t, ".subckt")
                && let Some(name) = t.split_whitespace().nth(1)
                && is_mos(name)
            {
                sub = Some(name.to_string());
                need_params = true;
                seen_y = false;
                if t.contains('=') {
                    out.push(format!("{} mm_ok=0", line.trim_end()));
                    need_params = false;
                    continue;
                }
            }
            out.push(line.to_string());
            continue;
        }
        let name = sub.clone().unwrap_or_default();
        if need_params && t.starts_with('+') {
            out.push(format!("{} mm_ok=0", line.trim_end()));
            need_params = false;
            continue;
        }
        if is_card(t, "ypsp103_va") {
            let model = t.split_whitespace().nth(6).unwrap_or_default();
            prefix = model.trim_end_matches("_psp").to_string();
            if prefix.is_empty() || !line.contains("w='w'") || !line.contains("l='l'") {
                bail!("{name}: the instance line is not the one the mismatch patch knows: {t}");
            }
            let agauss = |x: &str, sigma: &str| {
                format!("'if(mm_ok == 1, agauss({x}, {prefix}_{sigma}_mm, 1), {x})'")
            };
            out.push(
                line.replacen("w='w'", &format!("w={}", agauss("w", "dw")), 1)
                    .replacen("l='l'", &format!("l={}", agauss("l", "dl")), 1),
            );
            seen_y = true;
            continue;
        }
        if is_card(t, ".ends") {
            if !seen_y {
                bail!("{name}: no YPSP103_VA instance to give mismatch");
            }
            // The continuation lines of the instance end the last statement line.
            let at = out
                .iter()
                .rposition(|l| {
                    let l = l.trim();
                    !l.is_empty() && !l.starts_with('*')
                })
                .map_or(0, |i| i + 1);
            let extra = [
                format!(
                    "+ delvto='if(mm_ok == 1, agauss(0, {prefix}_delvto_mm/sqrt(m*l*w*1e12), 1), 0)'"
                ),
                format!(
                    "+ factuo='if(mm_ok == 1, agauss(1, {prefix}_factuo_mm/sqrt(m*l*w*1e12), 1), 1)'"
                ),
            ];
            for (i, e) in extra.into_iter().enumerate() {
                out.insert(at + i, e);
            }
            changed += 1;
            sub = None;
        }
        out.push(line.to_string());
    }
    if sub.is_some() {
        bail!("a MOS subcircuit has no .ends");
    }
    if changed == 0 {
        bail!("the file has no sg13_lv/hv_n/pmos subcircuit");
    }
    let mut head = vec![
        "* Written by CDePT: the PDK's Xyce MOS models with transistor mismatch (mm_ok=1 draws"
            .to_string(),
        "* w, l, delvto and factuo per instance, as the PDK's ngspice mismatch models do)."
            .to_string(),
    ];
    head.extend(params.iter().cloned());
    Ok((
        format!("{}\n{}\n", head.join("\n"), out.join("\n")),
        changed,
    ))
}

/// Make the folder `dest` with the files of the PDK's Xyce model folder `xyce_dir` (links)
/// in which the MOS files carry mismatch ([`patch_mos`]); the mismatch parameters come
/// from the ngspice model folder `ngspice_dir`. Safe to repeat. Returns `dest`.
pub fn overlay(xyce_dir: &Path, ngspice_dir: &Path, dest: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dest).wrap_err_with(|| format!("cannot create {}", dest.display()))?;
    let mut patched: Vec<(String, String)> = vec![];
    for (file, params_file) in MOS_FILES {
        let (src, par) = (xyce_dir.join(file), ngspice_dir.join(params_file));
        let text = std::fs::read_to_string(&src).wrap_err_with(|| {
            format!("Xyce mismatch needs {} of the Xyce models", src.display())
        })?;
        let params = std::fs::read_to_string(&par).wrap_err_with(|| {
            format!(
                "Xyce mismatch takes its sigmas from {} of the ngspice models",
                par.display()
            )
        })?;
        let params = mismatch_params(&params);
        if params.is_empty() {
            bail!("{} has no mismatch parameters", par.display());
        }
        let (new, _) = patch_mos(&text, &params).wrap_err_with(|| file.to_string())?;
        patched.push((file.to_string(), new));
    }
    for entry in std::fs::read_dir(xyce_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if patched.iter().any(|(f, _)| *f == name) {
            continue;
        }
        let link = dest.join(&name);
        if std::fs::read_link(&link).is_ok_and(|t| t == entry.path()) {
            continue;
        }
        let _ = std::fs::remove_file(&link);
        link_or_copy(&entry.path(), &link)?;
    }
    for (name, text) in patched {
        let file = dest.join(&name);
        if std::fs::read_to_string(&file).is_ok_and(|old| old == text) {
            continue;
        }
        let _ = std::fs::remove_file(&file);
        std::fs::write(&file, text).wrap_err_with(|| format!("cannot write {}", file.display()))?;
    }
    Ok(dest.to_path_buf())
}

#[cfg(unix)]
fn link_or_copy(src: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, link)
        .wrap_err_with(|| format!("cannot link {}", link.display()))
}

#[cfg(not(unix))]
fn link_or_copy(src: &Path, link: &Path) -> Result<()> {
    std::fs::copy(src, link)
        .map(|_| ())
        .wrap_err_with(|| format!("cannot copy {}", link.display()))
}

/// The model lines of a run with the files below `from` read from `to` instead.
pub fn redirect(lines: &[String], from: &Path, to: &Path) -> Vec<String> {
    let (from, to) = (from.display().to_string(), to.display().to_string());
    lines
        .iter()
        .map(|l| l.replace(&format!("{from}/"), &format!("{to}/")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const XYCE: &str = "* head\n.subckt sg13_lv_nmos d g s b\n+ w=0.35u l=0.34u ng=1 m=1 as=0 pre_layout=1\n\n.include sg13g2_moslv_parm.lib\n\nYPSP103_VA M1 d g s b sg13g2_lv_nmos_psp w='w' l='l' nf='ng' as='as' mult='m'\n+ dta=trise\n+ ngcon=2\n.ends\n\n.subckt nmoscl_2  VDD VSS\n+l=0.36u\nYPSP103_VA M1 VDD VSS VSS VSS nmoscl_psp w='w' l='l'\n.ends nmoscl_2\n";

    #[test]
    fn the_mos_subcircuit_gets_mm_ok_and_the_draws() {
        let params = vec![".param sg13g2_lv_nmos_dw_mm = 4e-9".to_string()];
        let (text, n) = patch_mos(XYCE, &params).unwrap();
        assert_eq!(n, 1);
        for want in [
            ".param sg13g2_lv_nmos_dw_mm = 4e-9\n",
            "+ w=0.35u l=0.34u ng=1 m=1 as=0 pre_layout=1 mm_ok=0\n",
            "w='if(mm_ok == 1, agauss(w, sg13g2_lv_nmos_dw_mm, 1), w)'",
            "l='if(mm_ok == 1, agauss(l, sg13g2_lv_nmos_dl_mm, 1), l)'",
            "+ ngcon=2\n+ delvto='if(mm_ok == 1, agauss(0, sg13g2_lv_nmos_delvto_mm/sqrt(m*l*w*1e12), 1), 0)'\n+ factuo='if(mm_ok == 1, agauss(1, sg13g2_lv_nmos_factuo_mm/sqrt(m*l*w*1e12), 1), 1)'\n.ends\n",
        ] {
            assert!(text.contains(want), "{want}\n{text}");
        }
        // The subcircuits that are not the transistors stay as they were.
        assert!(
            text.contains("nmoscl_psp w='w' l='l'\n.ends nmoscl_2"),
            "{text}"
        );
        // Nothing to patch is an error, not a silent no-op.
        assert!(patch_mos("* nothing\n.subckt a b\n.ends\n", &[]).is_err());
    }

    #[test]
    fn the_sigmas_come_from_the_ngspice_file() {
        let p = mismatch_params(
            "* c\n.param sg13g2_lv_nmos_delvto_mm  = 0.0039\n.param SWSOA = 0\n  .PARAM a=1\n",
        );
        assert_eq!(
            p,
            [".param sg13g2_lv_nmos_delvto_mm  = 0.0039", ".PARAM a=1"]
        );
    }

    #[test]
    fn the_overlay_links_the_rest_and_redirects_the_lines() {
        let d = tempfile::tempdir().unwrap();
        let (x, n, o) = (d.path().join("x"), d.path().join("n"), d.path().join("o"));
        std::fs::create_dir_all(&x).unwrap();
        std::fs::create_dir_all(&n).unwrap();
        for (f, p) in MOS_FILES {
            std::fs::write(x.join(f), XYCE).unwrap();
            std::fs::write(n.join(p), ".param sg13g2_lv_nmos_dw_mm = 4e-9\n").unwrap();
        }
        std::fs::write(x.join("cornerMOSlv.lib"), ".LIB mos_tt\n.ENDL\n").unwrap();
        overlay(&x, &n, &o).unwrap();
        overlay(&x, &n, &o).unwrap();
        assert!(
            std::fs::read_to_string(o.join("sg13g2_moslv_mod.lib"))
                .unwrap()
                .contains("mm_ok=0")
        );
        assert!(
            std::fs::read_to_string(o.join("cornerMOSlv.lib"))
                .unwrap()
                .contains("mos_tt")
        );
        let lines = vec![format!(".lib {}/cornerMOSlv.lib mos_tt", x.display())];
        assert_eq!(
            redirect(&lines, &x, &o),
            [format!(".lib {}/cornerMOSlv.lib mos_tt", o.display())]
        );
        // A missing ngspice file says what it was for.
        std::fs::remove_file(n.join("sg13g2_moshv_mismatch.lib")).unwrap();
        let e = overlay(&x, &n, &d.path().join("o2")).unwrap_err();
        assert!(format!("{e:#}").contains("sigmas"), "{e:#}");
    }
}
