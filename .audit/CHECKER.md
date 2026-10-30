# Gap checker brief

You check gaps that an auditor reported between SpaceEngine 0.991 and its Rust/Bevy port. Your job is to try to
PROVE EACH GAP WRONG. Be adversarial: auditors overstate, miss port code under another name, misread SE code, or
report something the port's README already lists as done. Only gaps that survive a real attempt to refute them stay.

Read `.audit/README.md` first: places, tools (Ghidra bridge helpers, `runport.sh`), and rules all apply to you too.
Audit only: never change the port, never write into the SpaceEngine install, never start SpaceEngine, never
`cargo build`. Write only `.audit/checks/<batch>.txt` and scratch files in `.audit/work/checks/<batch>/`.

## For each gap

1. Port side: search hard for an implementation (Grep `crates/` for the feature, its SE key names, synonyms; read
   README "Status"/"Controls" and `docs/`). If the claim is about behaviour, read the code path; run the port
   (`runport.sh`) when reading cannot settle it.
2. SE side: verify the cited evidence exists and says what the auditor claims (open the shader at the line, grep
   `main-def.cfg`, `eng-gui.cfg`, `re/strings_*.txt`, read the decompile in `.audit/work/dec/` or decompile it).
   A config key or string alone does not prove SE implements a feature the way claimed.
3. Out-of-scope items (VR, editors, online/Workshop, launcher) are REFUTED as out of scope.
4. Verdict:
   - `HOLDS`: evidence checks out on both sides.
   - `AMENDED`: a real gap, but the description, status (missing/partial/differs) or visibility was wrong; give the
     corrected text.
   - `REFUTED`: the port already does it, SE does not do it, or the evidence is wrong.
5. Effort to close it in the port: `S` (localised, about a day), `M` (a few days, one subsystem), `L` (a week or
   more, a new subsystem or further reverse engineering).

## Output

Write `.audit/checks/<batch>.txt` and return exactly its content, one line per gap, nothing else:

`<gap id> | HOLDS/AMENDED/REFUTED | effort S/M/L | visibility 1-5 | one-sentence reason with the decisive evidence | corrected feature text (AMENDED only, else -)`

Keep the visibility from the gap unless you have a reason to change it (say so in the reason).
