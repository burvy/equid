import Equid.Expr

/-!
# The 20 engine features, and why any weighting of them is a valid shortest-path metric

Same order and meaning as `FEATURES` in `engine/src/features.rs` (the index is the position).
Readings (the `Inputs` struct in the generated Rust):
`len` metres, `climb` metres climbed in the travel direction, `speed` m/s of the mode,
`grade` max grade %, `shade` share of direct sun blocked now, `rough` surface roughness,
`unlit` 0 lit / .5 unknown / 1 unlit, `flood`, `green`, `water` shares, `near_*` metres to the
nearest bench / toilets / water; counts `busy`, `lts`, `crossings`, `unsignalled`, `kerb`, `stops`.
-/

namespace Equid
open N

-- short names for building formulas
private def len : N := .nn .len
private def r (i : RealIn) : S := .real i
private def c (n : Int) : S := .const n 1
private def k (i : CountIn) : N := .count i

/-- `len / speed`: travel time in seconds. -/
def fTime : N := .div len (.nn .speed)
/-- metres climbed in the travel direction -/
def fClimb : N := .nn .climb
/-- `len * max((grade - 5) / 5, 0)`: 0 at <= 5 %, full length at 10 % -/
def fSteep : N := .mul len (.pos (.div (.sub (r .grade) (c 5)) (c 5)))
/-- length of stairs -/
def fSteps : N := .ite .steps len (.const 0 1)
/-- `len * clamp(roughness)` -/
def fRough : N := .mul len (.clamp01 (r .rough))
/-- `len * clamp(busy / 3)` -/
def fBusy : N := .mul len (.clamp01 (.div (.ofN (k .busy)) (c 3)))
/-- at night `len * clamp(unlit)`, else 0 -/
def fUnlit : N := .ite .night (.mul len (.clamp01 (r .unlit))) (.const 0 1)
/-- by day `len * (1 - clamp(shade))`, at night 0 -/
def fSun : N := .ite .night (.const 0 1) (.mul len (.compl01 (r .shade)))
/-- length not under a roof -/
def fExposed : N := .ite .covered (.const 0 1) len
/-- `len * clamp((lts - 1) / 3)`, unknown LTS 1/2 -/
def fStress : N := .mul len (.ite .hasLts (.clamp01 (.div (.sub (.ofN (k .lts)) (c 1)) (c 3))) (.const 1 2))
/-- `len * clamp(near_bench / 500)` -/
def fBenchGap : N := .mul len (.clamp01 (.div (r .nearBench) (c 500)))
def fToiletGap : N := .mul len (.clamp01 (.div (r .nearToilets) (c 500)))
def fWaterGap : N := .mul len (.clamp01 (.div (r .nearWater) (c 500)))
/-- `len * clamp((grade - 3) / 4) * (night ? 1 : clamp(shade))`: shaded hills stay icy -/
def fIcy : N := .mul (.mul len (.clamp01 (.div (.sub (r .grade) (c 3)) (c 4))))
  (.ite .night (.const 1 1) (.clamp01 (r .shade)))
/-- `len * clamp(flood)` -/
def fFlood : N := .mul len (.clamp01 (r .flood))
/-- `min(unsignalled, crossings) + 0.3 * max(crossings - min(unsignalled, crossings), 0) + junction`
(junction = street crossings derived from topology at the edge's ends, see `junction_charge_nonneg`) -/
def fCrossing : N :=
  let u := N.min (k .unsignalled) (k .crossings)
  .add (.add u (.mul (.const 3 10) (.pos (.sub (.ofN (k .crossings)) (.ofN u))))) (.nn .junction)
/-- length of road tagged without a sidewalk -/
def fNoSidewalk : N := .ite .noSidewalk len (.const 0 1)
/-- crossings with a raised kerb -/
def fKerb : N := k .kerb
/-- `len * (1 - clamp(max(green, water)))` when either is known, else 0 -/
def fDull : N := .mul len (.ite .hasGreenWater (.compl01 (.max (r .green) (r .water))) (.const 0 1))
/-- stop signs, give-ways and signals -/
def fStop : N := k .stops

/-- The feature vector, in engine order. -/
def features : List (String × N) :=
  [("time", fTime), ("climb", fClimb), ("steep", fSteep), ("steps", fSteps), ("rough", fRough),
   ("busy", fBusy), ("unlit", fUnlit), ("sun", fSun), ("exposed", fExposed), ("stress", fStress),
   ("bench_gap", fBenchGap), ("toilet_gap", fToiletGap), ("water_gap", fWaterGap), ("icy", fIcy),
   ("flood", fFlood), ("crossing", fCrossing), ("no_sidewalk", fNoSidewalk), ("kerb", fKerb),
   ("dull", fDull), ("stop", fStop)]

theorem features_length : features.length = 20 := rfl

/-- **Each of the 20 features is >= 0** on every edge the loader accepts. -/
theorem features_nonneg (env : Env) (h : env.Valid) : ∀ p ∈ features, 0 ≤ p.2.eval env :=
  fun p _ => N.eval_nonneg env h p.2

/-- The engine's edge cost: Σ aᵢ · fᵢ over the weights the user/AI picked (missing weights are 0). -/
def cost (env : Env) : List Rat → List (String × N) → Rat
  | a :: as, (_, f) :: fs => a * f.eval env + cost env as fs
  | _, _ => 0

/-- **Any non-negative weighting gives a non-negative edge cost.** Dijkstra's algorithm is correct
for non-negative edge costs, so every finder (walk, shade, night, bike, any AI-made metric with
weights >= 0) is a valid shortest-path metric. -/
theorem cost_nonneg (env : Env) (h : env.Valid) :
    ∀ (ws : List Rat), (∀ a ∈ ws, 0 ≤ a) → 0 ≤ cost env ws features := by
  suffices ∀ (fs : List (String × N)) (ws : List Rat), (∀ a ∈ ws, 0 ≤ a) → 0 ≤ cost env ws fs from
    this features
  intro fs
  induction fs with
  | nil => intro ws _; cases ws <;> simp [cost]
  | cons p fs ih =>
    intro ws hw
    cases ws with
    | nil => simp [cost]
    | cons a ws =>
      obtain ⟨_, f⟩ := p
      simp only [cost]
      have ha : 0 ≤ a := hw a (by simp)
      have h1 := Rat.mul_nonneg ha (N.eval_nonneg env h f)
      have h2 := ih ws (fun b hb => hw b (by simp [hb]))
      grind

/-- Object layers (`near:<label>`, task 015) add `w · len · near / 500` per label, `near` being a
distance (>= 0). Also >= 0 for w >= 0, so the total stays a valid metric. -/
theorem object_term_nonneg (w len near : Rat) (hw : 0 ≤ w) (hl : 0 ≤ len) (hn : 0 ≤ near) :
    0 ≤ w * len * near / 500 :=
  div_nonneg' (Rat.mul_nonneg (Rat.mul_nonneg hw hl) hn) (by decide)

/-! The derived `junction` reading is >= 0. At each end of a car-road edge the engine
(`Graph::derive_junctions`) charges `0.5 * (sum - max)` of the class weights (all >= 0) of the other
car roads meeting there. The largest is the road continuing straight. -/

def listMax : List Rat → Rat
  | [] => 0
  | x :: xs => max x (listMax xs)

theorem sum_nonneg_of (ws : List Rat) (h : ∀ w ∈ ws, 0 ≤ w) : 0 ≤ ws.sum := by
  induction ws with
  | nil => simp
  | cons x xs ih =>
    simp only [List.sum_cons]
    have := h x (by simp); have := ih (fun w hw => h w (by simp [hw])); grind

theorem listMax_le_sum (ws : List Rat) (h : ∀ w ∈ ws, 0 ≤ w) : listMax ws ≤ ws.sum := by
  induction ws with
  | nil => simp [listMax]
  | cons x xs ih =>
    simp only [listMax, List.sum_cons]
    have hx := h x (by simp)
    have hs := sum_nonneg_of xs (fun w hw => h w (by simp [hw]))
    have := ih (fun w hw => h w (by simp [hw]))
    grind

theorem junction_charge_nonneg (ws : List Rat) (h : ∀ w ∈ ws, 0 ≤ w) : 0 ≤ (1/2 : Rat) * (ws.sum - listMax ws) := by
  have := listMax_le_sum ws h
  exact Rat.mul_nonneg (by grind) (by grind)

/-! Spot checks: the formulas say what the docs say (by computation on concrete edges). -/

private def demo (grade shade : Rat) (night : Bool) : Env :=
  { nn := fun | .len => 100 | .climb => 3 | .speed => 2 | .junction => 0,
    real := fun | .grade => grade | .shade => shade | _ => 0,
    count := fun | .crossings => 2 | .unsignalled => 1 | _ => 0,
    flag := fun | .night => night | _ => false }

#guard fSteep.eval (demo 10 0 false) == 100  -- 10 % grade: full length
#guard fSteep.eval (demo 4 0 false) == 0     -- 4 %: nothing
#guard fIcy.eval (demo 7 (1/2) false) == 50  -- icy slope, half shaded
#guard fIcy.eval (demo 7 0 true) == 100      -- at night it stays icy
#guard fCrossing.eval (demo 0 0 false) == 13/10 -- 1 unsignalled + .3 signalled
#guard fTime.eval (demo 0 0 false) == 50

end Equid
