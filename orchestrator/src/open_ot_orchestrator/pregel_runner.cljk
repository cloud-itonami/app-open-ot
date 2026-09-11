(ns open-ot-orchestrator.pregel-runner
  "Idiomatic clj/cljc port of `pregel_runner.py` — the minimal Pregel BSP
  super-step runner for open-ot loops (ADR-2606280030, ADR-2605151200 §4.1).

  The IEC 61499 event-tick ≡ Pregel super-step contract lives here, kept
  isolated from any cell runtime. In the Python original each `CellSpec`
  carried a wasmtime `CellLoader`; that wasmtime binding has no babashka
  equivalent, so the cell runtime is modelled here as an **injectable seam**:
  a `cell-loader` is any map of four fns

    {:init!        (fn [params-bytes internal-size] -> nil)
     :tick         (fn [{:keys [event-in data-in-bytes ecc-state
                                super-step data-out-size]}]
                      -> {:next-ecc-state int :out-event-raw int
                          :data-out-bytes bytevec})
     :get-internal (fn [internal-size] -> bytevec)
     :set-internal! (fn [bytevec] -> nil)}

  The production loader is wasmtime (`cell_loader.py`, kept as `.py`); tests
  inject a pure-clj mock. The runner logic (dedup DIDs, per-step skip of
  unfed cells = single-task/row-driven trigger, monotonic checkpoint stream,
  restore + replay determinism) is byte-for-byte the Python semantics.

  `bytevec` = vector of unsigned-byte ints 0..255 (see `droop-codec`)."
  (:refer-clojure :exclude [replay]))

;; ---------------------------------------------------------------------------
;; data shapes (plain maps — idiomatic clj)
;; ---------------------------------------------------------------------------
;;
;; cell-spec  {:did str :loader <seam> :params-bytes bytevec
;;             :internal-size int :data-in-size int :data-out-size int
;;             :initial-ecc int}
;; step-input {:event-in-code int :data-in-bytes bytevec}
;; emission   {:cell-did str :next-ecc-state int :out-event-raw int
;;             :data-out-bytes bytevec}
;; checkpoint {:super-step int :ecc-states {did int} :internals {did bytevec}
;;             :emissions [emission]}

(defn make-loop-runner
  "Build a loop runner over an ordered seq of `cell-specs`. Returns an atom
  holding the mutable runner state (matches the Python stateful `LoopRunner`,
  since cell internal state lives behind the loader seam)."
  [cell-specs]
  (when (empty? cell-specs)
    (throw (ex-info "LoopRunner requires at least one cell" {})))
  (let [seen (volatile! #{})]
    (doseq [c cell-specs]
      (when (contains? @seen (:did c))
        (throw (ex-info (str "duplicate cell DID: " (:did c))
                        {:did (:did c)})))
      (vswap! seen conj (:did c))))
  (atom {:cells      (reduce (fn [m c] (assoc m (:did c) c)) {} cell-specs)
         ;; preserve insertion order for deterministic per-step iteration
         :order      (mapv :did cell-specs)
         :ecc-states (reduce (fn [m c] (assoc m (:did c) (:initial-ecc c 0)))
                             {} cell-specs)
         :super-step 0
         :checkpoints []}))

(defn- snapshot!
  "Append a checkpoint of the current state to the stream and return it."
  [runner emissions]
  (let [{:keys [cells order ecc-states super-step]} @runner
        internals (reduce (fn [m did]
                            (let [c (get cells did)]
                              (assoc m did ((get-in c [:loader :get-internal])
                                            (:internal-size c)))))
                          {} order)
        cp {:super-step super-step
            :ecc-states ecc-states
            :internals  internals
            :emissions  (vec emissions)}]
    (swap! runner update :checkpoints conj cp)
    cp))

(defn initialize!
  "Call `<cell>_init` for every cell, then snapshot the post-init state as
  super-step 0. Must run before `run-step!`."
  [runner]
  (let [{:keys [cells order]} @runner]
    (doseq [did order]
      (let [c (get cells did)]
        ((get-in c [:loader :init!]) (:params-bytes c) (:internal-size c))))
    (swap! runner assoc :super-step 0)
    (snapshot! runner [])))

(defn run-step!
  "Run one super-step. `inputs` is a map of did → step-input. Cells without an
  entry are skipped (retain ECC + internal) — the single-task/row-driven
  trigger pattern. Returns the resulting checkpoint."
  [runner inputs]
  (swap! runner update :super-step inc)
  (let [{:keys [cells order ecc-states super-step]} @runner
        emissions
        (reduce
          (fn [acc did]
            (if-not (contains? inputs did)
              acc
              (let [c   (get cells did)
                    inp (get inputs did)
                    {:keys [next-ecc-state out-event-raw data-out-bytes]}
                    ((get-in c [:loader :tick])
                     {:event-in      (:event-in-code inp)
                      :data-in-bytes (:data-in-bytes inp)
                      :ecc-state     (get ecc-states did)
                      :super-step    super-step
                      :data-out-size (:data-out-size c)})]
                (swap! runner assoc-in [:ecc-states did] next-ecc-state)
                (conj acc {:cell-did       did
                           :next-ecc-state next-ecc-state
                           :out-event-raw  out-event-raw
                           :data-out-bytes data-out-bytes}))))
          []
          order)]
    (snapshot! runner emissions)))

(defn restore-from-checkpoint!
  "Set each cell's internal bytes + ECC state from `cp`. Rejects a checkpoint
  whose cell set differs from the runner's. The new checkpoint stream starts
  fresh from the resumed point."
  [runner cp]
  (let [{:keys [cells]} @runner]
    (when (not= (set (keys (:internals cp))) (set (keys cells)))
      (throw (ex-info "checkpoint cell DIDs do not match runner cells"
                      {:checkpoint (set (keys (:internals cp)))
                       :runner     (set (keys cells))})))
    (doseq [[did c] cells]
      ((get-in c [:loader :set-internal!]) (get-in cp [:internals did]))
      (swap! runner assoc-in [:ecc-states did] (get-in cp [:ecc-states did])))
    (swap! runner assoc
           :super-step (:super-step cp)
           :checkpoints [cp])
    runner))

(defn checkpoints [runner] (:checkpoints @runner))
(defn cells [runner] (:cells @runner))
(defn super-step [runner] (:super-step @runner))

;; ---------------------------------------------------------------------------
;; replay helper
;; ---------------------------------------------------------------------------

(defn replay
  "Run `inputs-per-step` through a fresh runner from `runner-factory`. With
  `:resume-from` set, restore that checkpoint first and start at
  `:resume-inputs-offset`. Returns the full checkpoint stream."
  [runner-factory inputs-per-step
   & {:keys [resume-from resume-inputs-offset] :or {resume-inputs-offset 0}}]
  (let [runner (runner-factory)]
    (initialize! runner)
    (if (nil? resume-from)
      (doseq [inp inputs-per-step] (run-step! runner inp))
      (do
        (restore-from-checkpoint! runner resume-from)
        (doseq [inp (drop resume-inputs-offset inputs-per-step)]
          (run-step! runner inp))))
    (checkpoints runner)))
