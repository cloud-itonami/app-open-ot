(ns open-ot-orchestrator.core-test
  "clojure.test port of the wasm-free subset of `test_pregel_runner.py` +
  `test_checkpointer.py` (ADR-2606280030), plus codec parity tests for
  `droop_codec.cljc`.

  The Python tests that require a built `droop_p_f.wasm` (loaded via wasmtime)
  are NOT portable to babashka; they stay in the `.py` suite. Here the
  wasmtime `CellLoader` is replaced by an injected pure-clj MOCK droop cell
  that exercises the SAME runner / checkpointer logic (super-step semantics,
  per-step skip, monotonic stream, replay determinism, params-rev guard,
  multi-loop independence)."
  (:require [clojure.test :refer [deftest is testing run-tests]]
            [open-ot-orchestrator.droop-codec :as codec]
            [open-ot-orchestrator.pregel-runner :as pr]
            [open-ot-orchestrator.checkpointer :as cp]))

;; ---------------------------------------------------------------------------
;; pure-clj MOCK droop cell (replaces the wasmtime CellLoader seam)
;; ---------------------------------------------------------------------------

(defn- iabs [x] (if (neg? x) (- x) x))

(defn- mock-droop-loader
  "A deterministic stand-in for `droop_p_f.wasm` behind the runner's loader
  seam. Internal state (last setpoint) lives in an atom, mirroring how the
  real cell keeps state in wasm linear memory."
  []
  (let [internal (atom (vec (repeat codec/internal-size 0)))
        params   (atom nil)]
    {:init! (fn [params-bytes _internal-size]
              (reset! params (codec/unpack-params params-bytes))
              (reset! internal (codec/pack-internal
                                 {:last-setpoint-micro-kw 0 :initialized false})))
     :tick (fn [{:keys [data-in-bytes]}]
             (let [p   @params
                   din (codec/unpack-data-in data-in-bytes)
                   freq-err  (- (:grid-freq din) (:freq-nominal din))
                   dead-band (:dead-band-micro-hz p)
                   within?   (<= (iabs freq-err) dead-band)
                   p-rated-kw (quot (:p-rated-micro-kw p) 1000000)
                   delta-micro (if within? 0 (- (quot (* freq-err p-rated-kw) 100)))
                   setpoint-micro (+ (:current-p din) delta-micro)
                   data-out (codec/pack-data-out
                              {:p-setpoint setpoint-micro
                               :delta-p delta-micro
                               :freq-error freq-err
                               :dead-band-active within?
                               :saturated false})]
               (reset! internal (codec/pack-internal
                                  {:last-setpoint-micro-kw setpoint-micro
                                   :initialized true}))
               {:next-ecc-state (if within? 1 2)
                :out-event-raw 1
                :data-out-bytes data-out}))
     :get-internal (fn [size] (vec (take size @internal)))
     :set-internal! (fn [bytevec] (reset! internal (vec bytevec)))}))

(defn- build-freq-droop-loop
  "Port of microgrid_pregel.build_freq_droop_loop using mock loaders.
  `bess-assets` = seq of [did p-rated-kw]."
  [bess-assets & {:keys [cycle-period-ms] :or {cycle-period-ms 100}}]
  (pr/make-loop-runner
    (for [[did p-rated] bess-assets]
      {:did did
       :loader (mock-droop-loader)
       :params-bytes (codec/pack-droop-params
                       {:p-rated-kw p-rated :p-min-kw 0 :p-max-kw p-rated
                        :droop-pct 5.0 :dead-band-hz 0.2
                        :cycle-period-ms cycle-period-ms})
       :internal-size codec/internal-size
       :data-in-size codec/data-in-size
       :data-out-size codec/data-out-size
       :initial-ecc 0})))

(defn- step-freq-droop [runner grid-freq-hz current-p-per-asset]
  (let [inputs (reduce (fn [m did]
                         (assoc m did
                                {:event-in-code 0
                                 :data-in-bytes (codec/pack-droop-data-in
                                                  {:grid-freq-hz grid-freq-hz
                                                   :freq-nominal-hz 50.0
                                                   :current-p-kw (get current-p-per-asset did 0.0)})}))
                       {} (keys (pr/cells runner)))]
    (pr/run-step! runner inputs)))

(defn- cohort-total-delta-kw [checkpoint]
  (reduce (fn [t em]
            (+ t (:delta-p-kw (codec/unpack-droop-data-out (:data-out-bytes em)))))
          0.0
          (:emissions checkpoint)))

;; ---------------------------------------------------------------------------
;; codec parity
;; ---------------------------------------------------------------------------

(deftest test-params-byte-layout
  (testing "pack-droop-params matches the Python struct '<iiiiiI' bytes"
    ;; 1000 kW rated, droop 5.0%, deadband 0.2 Hz, cycle 100 ms
    (let [b (codec/pack-droop-params
              {:p-rated-kw 1000.0 :p-min-kw 0 :p-max-kw 1000.0
               :droop-pct 5.0 :dead-band-hz 0.2 :cycle-period-ms 100})]
      (is (= 24 (count b)))
      ;; p_rated 1e9, p_min 0, p_max 1e9, droop_permille 50,
      ;; dead_band_micro_hz 200000, cycle_period_ms 100
      (is (= "00ca9a3b00000000" (codec/hex (subvec b 0 8))))
      (is (= "00ca9a3b" (codec/hex (subvec b 8 12))))
      (is (= "32000000" (codec/hex (subvec b 12 16))))
      (is (= "400d0300" (codec/hex (subvec b 16 20))))
      (is (= "64000000" (codec/hex (subvec b 20 24)))))))

(deftest test-codec-round-trips
  (testing "DataIn / DataOut / Params / Internal pack→unpack are identity"
    (let [din {:grid-freq 50500000 :freq-nominal 50000000 :current-p 800000000
               :freq-quality 0 :enable true}]
      (is (= din (codec/unpack-data-in (codec/pack-data-in din)))))
    (let [dout {:p-setpoint 795000000 :delta-p -5000000 :freq-error 500000
                :dead-band-active false :saturated false}]
      (is (= dout (codec/unpack-data-out (codec/pack-data-out dout)))))
    (let [intl {:last-setpoint-micro-kw -123456 :initialized true}]
      (is (= intl (codec/unpack-internal (codec/pack-internal intl)))))
    (testing "negative i32 round-trips (two's complement)"
      (is (= -1 (:delta-p (codec/unpack-data-out
                            (codec/pack-data-out {:p-setpoint 0 :delta-p -1
                                                  :freq-error -1
                                                  :dead-band-active false
                                                  :saturated false}))))))))

;; ---------------------------------------------------------------------------
;; pregel runner (mock-cell)
;; ---------------------------------------------------------------------------

(deftest test-loop-runner-rejects-duplicate-dids
  (let [c {:did "x" :loader (mock-droop-loader) :params-bytes []
           :internal-size 0 :data-in-size 0 :data-out-size 0}]
    (is (thrown-with-msg? clojure.lang.ExceptionInfo #"duplicate cell DID"
          (pr/make-loop-runner [c c])))))

(deftest test-empty-loop-rejected
  (is (thrown? clojure.lang.ExceptionInfo (pr/make-loop-runner []))))

(deftest test-single-cell-step
  (let [r (build-freq-droop-loop [["did:droop-test" 1000.0]])]
    (pr/initialize! r)
    (let [cp (step-freq-droop r 50.0 {"did:droop-test" 500.0})]
      (is (= 1 (:super-step cp)))
      (is (= 1 (count (:emissions cp))))
      (let [out (codec/unpack-droop-data-out (:data-out-bytes (first (:emissions cp))))]
        (is (:dead-band-active out))
        (is (= 0.0 (:delta-p-kw out)))))))

(deftest test-two-cells-cohort-response
  (let [r (build-freq-droop-loop [["a" 1000.0] ["b" 500.0]])]
    (pr/initialize! r)
    (let [cp (step-freq-droop r 50.5 {"a" 800.0 "b" 400.0})]
      (is (neg? (cohort-total-delta-kw cp)))
      (doseq [em (:emissions cp)]
        (is (= 1 (:out-event-raw em)))))))

(deftest test-single-task-skips-unaffected-cells
  (let [r (build-freq-droop-loop [["a" 1000.0] ["b" 500.0]])]
    (pr/initialize! r)
    (let [inputs {"a" {:event-in-code 0
                       :data-in-bytes (codec/pack-droop-data-in
                                        {:grid-freq-hz 50.5 :freq-nominal-hz 50.0
                                         :current-p-kw 800.0})}}
          cp (pr/run-step! r inputs)]
      (is (= 1 (:super-step cp)))
      (is (= #{"a"} (set (map :cell-did (:emissions cp)))))
      (is (= #{"a" "b"} (set (keys (:ecc-states cp)))))
      (is (= #{"a" "b"} (set (keys (:internals cp))))))))

(deftest test-checkpoint-stream-monotonic
  (let [r (build-freq-droop-loop [["a" 1000.0]])]
    (pr/initialize! r)
    (doseq [f [50.0 50.1 50.3 50.5 50.3 50.0]]
      (step-freq-droop r f {"a" 800.0}))
    (is (= 7 (count (pr/checkpoints r))))
    (doseq [[i cp] (map-indexed vector (pr/checkpoints r))]
      (is (= i (:super-step cp))))))

(deftest test-replay-determinism-after-checkpoint-restore
  (let [schedule [[50.000 {"a" 800.0}]
                  [50.300 {"a" 800.0}]
                  [50.500 {"a" 800.0}]
                  [50.300 {"a" 750.0}]
                  [50.050 {"a" 720.0}]]
        factory #(build-freq-droop-loop [["a" 1000.0]])
        r1 (factory)]
    (pr/initialize! r1)
    (doseq [[f p] schedule] (step-freq-droop r1 f p))
    (let [full-outputs   (mapv #(codec/unpack-droop-data-out
                                  (:data-out-bytes (first (:emissions %))))
                               (rest (pr/checkpoints r1)))
          full-internals (mapv #(get-in % [:internals "a"]) (rest (pr/checkpoints r1)))
          pivot (nth (pr/checkpoints r1) 2)
          r2 (factory)]
      (pr/initialize! r2)
      (pr/restore-from-checkpoint! r2 pivot)
      (doseq [[f p] (drop 2 schedule)] (step-freq-droop r2 f p))
      (let [resumed-outputs   (mapv #(codec/unpack-droop-data-out
                                       (:data-out-bytes (first (:emissions %))))
                                    (rest (pr/checkpoints r2)))
            resumed-internals (mapv #(get-in % [:internals "a"]) (rest (pr/checkpoints r2)))]
        (is (= (subvec full-outputs 2) resumed-outputs))
        (is (= (subvec full-internals 2) resumed-internals))))))

(deftest test-restore-rejects-mismatched-cell-set
  (let [ra (build-freq-droop-loop [["a" 1000.0]])]
    (pr/initialize! ra)
    (step-freq-droop ra 50.0 {"a" 500.0})
    (let [cp (last (pr/checkpoints ra))
          rb (build-freq-droop-loop [["b" 500.0]])]
      (pr/initialize! rb)
      (is (thrown? clojure.lang.ExceptionInfo
            (pr/restore-from-checkpoint! rb cp))))))

;; ---------------------------------------------------------------------------
;; checkpointer (in-memory store = kotoba-Datom-log seam)
;; ---------------------------------------------------------------------------

(deftest test-store-starts-empty
  (let [s (cp/in-memory-store)]
    (is (= 0 (cp/count-checkpoints s nil)))
    (is (nil? (cp/latest-checkpoint s "did:loop:absent")))))

(deftest test-signal-change-insert-returns-row-id
  (let [s (cp/in-memory-store)
        r1 (cp/record-signal! s "did:signal:freq" 50000000 "good")
        r2 (cp/record-signal! s "did:signal:freq" 50001000 "good")]
    (is (= r2 (inc r1)))))

(deftest test-signal-change-records-loop-did-affected
  (let [s (cp/in-memory-store)
        rid (cp/record-signal! s "did:signal:freq-50hz" 50500000 "good"
                               ["did:loop:freq-droop" "did:loop:islanding"])]
    (is (>= rid 1))))

(deftest test-write-then-read-round-trip
  (let [r (build-freq-droop-loop [["a" 1000.0]])
        _ (pr/initialize! r)
        s (cp/in-memory-store)
        loop-did "did:loop:freq-droop-test"]
    (cp/write-runner-checkpoint! s loop-did (pr/cells r) (first (pr/checkpoints r)))
    (doseq [f [50.000 50.300 50.500]]
      (let [c (step-freq-droop r f {"a" 800.0})]
        (cp/write-runner-checkpoint! s loop-did (pr/cells r) c)))
    (is (= 4 (cp/count-checkpoints s loop-did)))
    (let [rows (cp/list-checkpoints s loop-did)]
      (is (= [0 1 2 3] (mapv :super-step rows)))
      ;; internals preserved byte-identical through json+base64
      (doseq [[row src] (map vector rows (pr/checkpoints r))]
        (is (= (:internals row) (:internals src)))
        (is (= (:ecc-states row) (:ecc-states src)))))))

(deftest test-resume-validates-params-rev-mismatch
  (let [s (cp/in-memory-store)
        loop-did "did:loop:rev-test"
        r1 (build-freq-droop-loop [["a" 1000.0]])]
    (pr/initialize! r1)
    (cp/write-runner-checkpoint! s loop-did (pr/cells r1) (first (pr/checkpoints r1)))
    (doseq [[f p] [[50.0 800.0] [50.3 800.0]]]
      (cp/write-runner-checkpoint! s loop-did (pr/cells r1)
                                   (step-freq-droop r1 f {"a" p})))
    ;; resume with different params (500 kW) → params_rev differs
    (let [r2 (build-freq-droop-loop [["a" 500.0]])]
      (pr/initialize! r2)
      (is (thrown-with-msg? clojure.lang.ExceptionInfo #"params_rev mismatch"
            (cp/restore-runner-from-checkpointer s loop-did (pr/cells r2)))))))

(deftest test-multi-loop-independence
  (let [s (cp/in-memory-store)
        loop-a "did:loop:a"
        loop-b "did:loop:b"
        ra (build-freq-droop-loop [["x" 1000.0]])
        rb (build-freq-droop-loop [["x" 500.0]])]
    (pr/initialize! ra)
    (pr/initialize! rb)
    (doseq [f [50.0 50.3 50.5]]
      (cp/write-runner-checkpoint! s loop-a (pr/cells ra)
                                   (step-freq-droop ra f {"x" 800.0})))
    (doseq [f [50.1 50.2]]
      (cp/write-runner-checkpoint! s loop-b (pr/cells rb)
                                   (step-freq-droop rb f {"x" 400.0})))
    (is (= 3 (cp/count-checkpoints s loop-a)))
    (is (= 2 (cp/count-checkpoints s loop-b)))
    (is (= [1 2 3] (mapv :super-step (cp/list-checkpoints s loop-a))))
    (is (= [1 2] (mapv :super-step (cp/list-checkpoints s loop-b))))))

(defn -main [& _]
  (let [{:keys [fail error]} (run-tests 'open-ot-orchestrator.core-test)]
    (System/exit (if (pos? (+ (or fail 0) (or error 0))) 1 0))))
