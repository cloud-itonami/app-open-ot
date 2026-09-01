;; Walk docs/operator-quickstart.md and check each of its claims against what
;; the repository actually does, right now.
;;
;;   nbb docs/verify-quickstart.cljs --root .
;;
;; Exit 0 = every claim held. 1 = a claim was contradicted. 2 = REFUSED — could
;; not measure. 2 is deliberately neither 0 nor 1: "I could not check" must not
;; come back looking like "I checked and it was fine".
;;
;; §2 of the quickstart describes what pytest reports on a machine with **no
;; built cell artefacts**. When the artefacts are present those numbers are
;; inapplicable, not wrong — this reports them as n/a and never scores them as
;; passes.
(ns verify-quickstart
  (:require ["fs" :as fs]
            ["child_process" :as cp]
            [clojure.string :as str]))

(def root (or (second (drop-while #(not= "--root" %) (js->clj js/process.argv))) nil))
(when-not root
  (println "REFUSED: --root <dir> is required")
  (js/process.exit 2))

(def qs-path (str root "/docs/operator-quickstart.md"))
(when-not (fs/existsSync qs-path)
  (println "REFUSED: no quickstart at" qs-path)
  (js/process.exit 2))
(def qs (fs/readFileSync qs-path "utf8"))

(defn sh [cwd cmd]
  (try
    (let [out (cp/execSync cmd #js {:cwd cwd :encoding "utf8"
                                    :stdio #js ["ignore" "pipe" "pipe"]
                                    :timeout 300000})]
      {:exit 0 :out out})
    (catch :default e
      {:exit (or (.-status e) :signal)
       :out (str (some-> (.-stdout e) str) (some-> (.-stderr e) str))})))

(def results (atom []))
(def skipped (atom []))
(defn check! [id ok? detail]
  (swap! results conj {:id id :ok ok? :detail detail})
  (println (if ok? "  ok  " "  FAIL") id "—" detail))
(defn not-applicable! [id detail]
  (swap! skipped conj {:id id :detail detail})
  (println "  n/a " id "—" detail))

;; Are the host-harness cell artefacts built? §2's numbers only describe the
;; state where they are not.
(def wasm-dir (str root "/cells/target/wasm32-unknown-unknown/release"))
(def cells-built?
  (and (fs/existsSync wasm-dir)
       (boolean (seq (filter #(str/ends-with? % ".wasm") (fs/readdirSync wasm-dir))))))

;; ── claim 1: the quickstart must not promise a step it never ran ───────────
;; Every fenced bash block that the prose introduces as runnable is walked.

(println "── walking docs/operator-quickstart.md against" root)

;; §1 — bb run_tests.clj
(let [r (sh (str root "/orchestrator") "bb run_tests.clj")
      claimed-lines ["Ran 16 tests containing 53 assertions." "0 failures, 0 errors."]]
  (check! :qs1/exit-zero (= 0 (:exit r)) (str "bb run_tests.clj exit=" (:exit r)))
  (doseq [l claimed-lines]
    (check! (keyword "qs1" (str "line/" (hash l)))
            (str/includes? (:out r) l)
            (str "output contains " (pr-str l))))
  (check! :qs1/quickstart-quotes-it
          (every? #(str/includes? qs %) claimed-lines)
          "quickstart quotes those exact lines"))

;; §1 — bb test is claimed to print the same three lines
(let [r (sh (str root "/orchestrator") "bb test")]
  (check! :qs1/bb-task-exit-zero (= 0 (:exit r)) (str "bb test exit=" (:exit r)))
  (check! :qs1/bb-task-same-output
          (str/includes? (:out r) "Ran 16 tests containing 53 assertions.")
          "bb test prints the same assertion line"))

;; §2 — uv sync --frozen then pytest
(let [r (sh (str root "/orchestrator") "uv sync --frozen")]
  (check! :qs2/uv-sync (= 0 (:exit r)) (str "uv sync --frozen exit=" (:exit r))))

(let [r (sh (str root "/orchestrator") "uv run --frozen pytest -q --no-header --tb=no")
      out (:out r)
      m (re-find #"(\d+) failed, (\d+) passed, (\d+) skipped" out)]
  (if cells-built?
    (not-applicable! :qs2/counts "cells are built — §2 describes the cargo-less state")
    (do
      (check! :qs2/nonzero-exit (not= 0 (:exit r))
              (str "pytest exits nonzero without cells built, exit=" (:exit r)))
      (check! :qs2/summary-parsed (some? m) (str "summary line present: " (pr-str (first m))))
      (when m
        (let [[_ f p s] m]
          (check! :qs2/counts (= [f p s] ["26" "14" "25"])
                  (str "actual " f "/" p "/" s " failed/passed/skipped"))
          (check! :qs2/quickstart-quotes-counts
                  (str/includes? qs (str f " failed, " p " passed, " s " skipped"))
                  "quickstart quotes those counts verbatim"))))))

;; §2 — per-module attribution table in the quickstart
(let [r (sh (str root "/orchestrator") "uv run --frozen pytest --no-header --tb=no -rA -q")
      out (:out r)
      _ (when cells-built?
          (not-applicable! :qs2/failed-modules "cells are built — §2 describes the cargo-less state"))
      tally (reduce (fn [acc line]
                      (if-let [[_ st mod] (re-find #"^(PASSED|FAILED) (tests/[^:]+)" line)]
                        (update-in acc [st mod] (fnil inc 0)) acc))
                    {} (str/split-lines out))
      failed (get tally "FAILED")]
  (when-not cells-built?
   (check! :qs2/failed-modules
          (= failed {"tests/test_microgrid_bess_langgraph.py" 7
                     "tests/test_microgrid_pv_mppt_langgraph.py" 7
                     "tests/test_microgrid_volt_var_langgraph.py" 7
                     "tests/test_microgrid_islanding_blackstart_langgraph.py" 5})
          (str "failing modules/counts = " (pr-str failed)))
   (doseq [[m n] failed]
     (check! (keyword "qs2" (str "table-row/" (str/replace m #"[^a-z0-9]" "-")))
             (str/includes? qs (str "`" m "` (" n ")"))
             (str "quickstart's table names " m " with (" n ")")))))

;; §2 — the targeted command the quickstart offers
(let [r (sh (str root "/orchestrator")
            "uv run --frozen pytest -q --no-header --tb=no tests/test_generated_byte_equivalence.py tests/test_checkpointer.py")
      m (re-find #"(\d+) passed, (\d+) skipped" (:out r))]
  (check! :qs2/targeted-exit-zero (= 0 (:exit r)) (str "targeted pytest exit=" (:exit r)))
  (if cells-built?
    (not-applicable! :qs2/targeted-counts "cells are built — §2 describes the cargo-less state")
    (do (check! :qs2/targeted-counts (= (vec (rest m)) ["13" "4"])
                (str "targeted run = " (pr-str (first m))))
        (check! :qs2/targeted-quoted
                (str/includes? qs "`13 passed, 4 skipped`")
                "quickstart quotes the targeted result"))))

;; ── claim 3: the skip-guard asymmetry the quickstart blames the failures on ──
(let [guarded (->> (fs/readdirSync (str root "/orchestrator/tests"))
                   (filter #(str/starts-with? % "test_"))
                   (filter #(str/includes?
                             (fs/readFileSync (str root "/orchestrator/tests/" %) "utf8")
                             "requires_wasm = pytest.mark.skipif")))]
  (check! :guard/five-modules (= 5 (count guarded))
          (str (count guarded) " modules declare requires_wasm: " (pr-str (sort guarded))))
  (check! :guard/quickstart-says-five
          (str/includes? qs "Five\ntest modules declare")
          "quickstart says five modules declare the guard"))

;; ── claim 4: cell inventory the README publishes ────────────────────────────
(let [cells (->> (fs/readdirSync (str root "/cells"))
                 (filter #(fs/existsSync (str root "/cells/" % "/src/lib.rs"))))
      bfb (remove #{"openot-bfb-rs"} cells)
      readme (fs/readFileSync (str root "/README.md") "utf8")
      counts (into {} (for [c bfb]
                        [c (count (re-seq #"(?m)^\s*#\[test\]"
                                          (fs/readFileSync (str root "/cells/" c "/src/lib.rs") "utf8")))]))]
  (check! :cells/nine (= 9 (count bfb)) (str (count bfb) " BFB cell crates"))
  (check! :cells/readme-says-nine
          (str/includes? readme "Nine BFB cells plus the shared `openot-bfb-rs` trait crate")
          "README says nine")
  (doseq [[c n] (sort counts)]
    (check! (keyword "cells" (str "row/" c))
            (str/includes? readme (str "| `cells/" c "` | " n " |"))
            (str "README row for " c " claims " n)))
  (check! :cells/total-78
          (str/includes? qs (str "contain " (reduce + (vals counts)) " `#[test]` functions"))
          (str "quickstart's total = " (reduce + (vals counts)))))

;; ── claim 5: paths the README/CLAUDE.md link to must exist here ─────────────
(let [readme (fs/readFileSync (str root "/README.md") "utf8")
      rel (->> (re-seq #"\]\(([^)h][^)]*)\)" readme)
               (map second)
               (remove #(str/starts-with? % "#"))
               distinct)
      missing (remove #(fs/existsSync (str root "/" (first (str/split % #"#")))) rel)]
  (check! :links/relative-resolve (empty? missing)
          (str (count rel) " relative links, missing: " (pr-str missing))))

(let [docs (map #(fs/readFileSync (str root "/" %) "utf8") ["README.md" "CLAUDE.md"])
      stale (filter #(re-find #"60-apps/etzhayyim-project-open-ot/(SPEC|cells|risk1|orchestrator|cad-spec|nixos|PROTOTYPE)" %) docs)]
  (check! :links/no-monorepo-cd (empty? stale)
          (str (count stale) " of README.md/CLAUDE.md still give monorepo-relative paths as if local")))

;; ── verdict ─────────────────────────────────────────────────────────────────
(let [rs @results
      bad (remove :ok rs)]
  (println)
  (println (str "CHECKED\t" (count rs) "\tN/A\t" (count @skipped)
                (when cells-built? "\t(cells are built)")))
  ;; An evidence floor. Zero checks, or a check set gutted down to the handful
  ;; that survive when everything is n/a, must not be reported as a pass.
  (when (< (count rs) 20)
    (println (str "REFUSED: only " (count rs)
                  " checks ran — too few to call this a pass"))
    (js/process.exit 2))
  (if (seq bad)
    (do (println (str "FAIL\t" (count bad) " claim(s) contradicted"))
        (doseq [b bad] (println "  ✗" (:id b) "—" (:detail b)))
        (js/process.exit 1))
    (do (println "PASS\tevery claim in the quickstart held")
        (js/process.exit 0))))
