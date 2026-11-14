/-!
# A tiny formula language for edge features

Every engine feature is a formula over the readings of one edge (its length, grade, shade, ...).
We write those formulas once, here, as Lean terms. Two things are then done with the *same* terms:

* `N.eval` gives their meaning as exact rational numbers, and `N.eval_nonneg` proves the
  result is never negative;
* `N.rust` prints them as Rust (`f32`) code for `engine/src/metrics_gen.rs`.

The trick that makes the proof one line per feature: there are two kinds of formulas.
`S` ("signed") may be any number. `N` ("non-negative") only has constructors that keep a
value >= 0: sums, products and quotients of non-negative things, `max(s, 0)`, `clamp(s, 0, 1)`,
`1 - clamp(s, 0, 1)`, if-then-else of two non-negative things, ... So *a formula that type-checks
as `N` is non-negative*, whatever the signed readings are. The only assumption is about the
`NnIn` readings (length, climb, speed), which the engine's graph loader checks are >= 0.
-/

namespace Equid

/-- Readings known to be >= 0: length, climb, descent (checked by `Graph::validate`), speed (a positive
constant) and the junction crossings the loader derives (>= 0 by `junction_charge_nonneg` in Features.lean). -/
inductive NnIn | len | climb | speed | junction
  deriving Repr, DecidableEq

/-- Readings that may be any number: nothing is assumed about them. -/
inductive RealIn | grade | shade | rough | unlit | flood | green | water | nearBench | nearToilets | nearWater
  deriving Repr, DecidableEq

/-- Small whole numbers (u8 in Rust): counts, and the bike traffic-stress level. -/
inductive CountIn | busy | lts | crossings | unsignalled | kerb | stops
  deriving Repr, DecidableEq

/-- Yes/no readings. -/
inductive FlagIn | night | steps | covered | noSidewalk | hasLts | hasGreenWater
  deriving Repr, DecidableEq

/-- Field names in the generated Rust `Inputs` struct. -/
def NnIn.rust : NnIn → String
  | .len => "len" | .climb => "climb" | .speed => "speed" | .junction => "junction"
def RealIn.rust : RealIn → String
  | .grade => "grade" | .shade => "shade" | .rough => "rough" | .unlit => "unlit" | .flood => "flood"
  | .green => "green" | .water => "water" | .nearBench => "near_bench" | .nearToilets => "near_toilets"
  | .nearWater => "near_water"
def CountIn.rust : CountIn → String
  | .busy => "busy" | .lts => "lts" | .crossings => "crossings" | .unsignalled => "unsignalled"
  | .kerb => "kerb" | .stops => "stops"
def FlagIn.rust : FlagIn → String
  | .night => "night" | .steps => "steps" | .covered => "covered" | .noSidewalk => "no_sidewalk"
  | .hasLts => "has_lts" | .hasGreenWater => "has_green_water"

/-- One edge, traversed in one direction at one time of day. -/
structure Env where
  nn : NnIn → Rat
  real : RealIn → Rat
  count : CountIn → Nat
  flag : FlagIn → Bool

/-- What the engine's loader guarantees. Nothing about `real`, `count` or `flag`. -/
def Env.Valid (env : Env) : Prop := ∀ i, 0 ≤ env.nn i

mutual
/-- Signed formulas: any value. -/
inductive S where
  | real : RealIn → S
  | ofN : N → S
  /-- the integer constant `n / d` -/
  | const : Int → Nat → S
  | add : S → S → S
  | sub : S → S → S
  | mul : S → S → S
  | div : S → S → S
  | min : S → S → S
  | max : S → S → S
/-- Non-negative formulas: every constructor keeps the value >= 0. -/
inductive N where
  | nn : NnIn → N
  | count : CountIn → N
  /-- the constant `n / d`, n and d natural numbers -/
  | const : Nat → Nat → N
  | add : N → N → N
  | mul : N → N → N
  | div : N → N → N
  | min : N → N → N
  | max : N → N → N
  /-- `max(s, 0)` -/
  | pos : S → N
  /-- `clamp(s, 0, 1)` -/
  | clamp01 : S → N
  /-- `1 - clamp(s, 0, 1)` -/
  | compl01 : S → N
  | ite : FlagIn → N → N → N
end

def clamp01 (x : Rat) : Rat := max 0 (min x 1)

mutual
def S.eval (env : Env) : S → Rat
  | .real i => env.real i
  | .ofN e => e.eval env
  | .const n d => (n : Rat) / (d : Rat)
  | .add a b => a.eval env + b.eval env
  | .sub a b => a.eval env - b.eval env
  | .mul a b => a.eval env * b.eval env
  | .div a b => a.eval env / b.eval env
  | .min a b => Min.min (a.eval env) (b.eval env)
  | .max a b => Max.max (a.eval env) (b.eval env)
def N.eval (env : Env) : N → Rat
  | .nn i => env.nn i
  | .count i => (env.count i : Rat)
  | .const n d => (n : Rat) / (d : Rat)
  | .add a b => a.eval env + b.eval env
  | .mul a b => a.eval env * b.eval env
  | .div a b => a.eval env / b.eval env
  | .min a b => Min.min (a.eval env) (b.eval env)
  | .max a b => Max.max (a.eval env) (b.eval env)
  | .pos s => Max.max (s.eval env) 0
  | .clamp01 s => clamp01 (s.eval env)
  | .compl01 s => 1 - clamp01 (s.eval env)
  | .ite f a b => if env.flag f then a.eval env else b.eval env
end

/-! ## The proof -/

theorem inv_nonneg' {b : Rat} (hb : 0 ≤ b) : 0 ≤ b⁻¹ := by
  by_cases h : b = 0
  · subst h; rw [Rat.inv_zero]; exact Rat.le_refl
  · exact Rat.le_of_lt (Rat.inv_pos.mpr (by grind))

theorem div_nonneg' {a b : Rat} (ha : 0 ≤ a) (hb : 0 ≤ b) : 0 ≤ a / b := by
  rw [Rat.div_def]; exact Rat.mul_nonneg ha (inv_nonneg' hb)

theorem natCast_nonneg' (n : Nat) : (0 : Rat) ≤ (n : Rat) := by exact_mod_cast Nat.zero_le n

theorem clamp01_nonneg (x : Rat) : 0 ≤ clamp01 x := by unfold clamp01; grind
theorem clamp01_le_one (x : Rat) : clamp01 x ≤ 1 := by unfold clamp01; grind

/-- **Every `N` formula is >= 0** on every edge the loader accepts, whatever its signed readings,
counts and flags are. -/
theorem N.eval_nonneg (env : Env) (h : env.Valid) : ∀ e : N, 0 ≤ e.eval env
  | .nn i => by simp only [N.eval]; exact h i
  | .count i => by simp only [N.eval]; exact natCast_nonneg' _
  | .const n d => by simp only [N.eval]; exact div_nonneg' (natCast_nonneg' n) (natCast_nonneg' d)
  | .add a b => by
      simp only [N.eval]; have := N.eval_nonneg env h a; have := N.eval_nonneg env h b; grind
  | .mul a b => by
      simp only [N.eval]; exact Rat.mul_nonneg (N.eval_nonneg env h a) (N.eval_nonneg env h b)
  | .div a b => by
      simp only [N.eval]; exact div_nonneg' (N.eval_nonneg env h a) (N.eval_nonneg env h b)
  | .min a b => by
      simp only [N.eval]; have := N.eval_nonneg env h a; have := N.eval_nonneg env h b; grind
  | .max a b => by
      simp only [N.eval]; have := N.eval_nonneg env h a; grind
  | .pos s => by simp only [N.eval]; grind
  | .clamp01 s => by simp only [N.eval]; exact clamp01_nonneg _
  | .compl01 s => by simp only [N.eval]; have := clamp01_le_one (s.eval env); grind
  | .ite f a b => by
      simp only [N.eval]; have := N.eval_nonneg env h a; have := N.eval_nonneg env h b
      split <;> assumption

/-! ## Printing as Rust (f32). Each operation maps to the same f32 operation, in the same order. -/

def lit (n : Int) : String := s!"{n}.0"

mutual
def S.rust : S → String
  | .real i => s!"i.{i.rust}"
  | .ofN e => e.rust
  | .const n d => if d == 1 then lit n else s!"({lit n} / {lit d})"
  | .add a b => s!"({a.rust} + {b.rust})"
  | .sub a b => s!"({a.rust} - {b.rust})"
  | .mul a b => s!"({a.rust} * {b.rust})"
  | .div a b => s!"({a.rust} / {b.rust})"
  | .min a b => s!"{a.rust}.min({b.rust})"
  | .max a b => s!"{a.rust}.max({b.rust})"
def N.rust : N → String
  | .nn i => s!"i.{i.rust}"
  | .count i => s!"i.{i.rust}"
  | .const n d => if d == 1 then lit n else s!"({lit n} / {lit d})"
  | .add a b => s!"({a.rust} + {b.rust})"
  | .mul a b => s!"({a.rust} * {b.rust})"
  | .div a b => s!"({a.rust} / {b.rust})"
  | .min a b => s!"{a.rust}.min({b.rust})"
  | .max a b => s!"{a.rust}.max({b.rust})"
  | .pos s => s!"{s.rust}.max(0.0)"
  | .clamp01 s => s!"{s.rust}.clamp(0.0, 1.0)"
  | .compl01 s => s!"(1.0 - {s.rust}.clamp(0.0, 1.0))"
  | .ite f a b => s!"(if i.{f.rust} \{ {a.rust} } else \{ {b.rust} })"
end

end Equid
