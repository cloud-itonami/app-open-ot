(ns open-ot-orchestrator.checkpointer
  "Idiomatic clj/cljc port of `checkpointer.py` — the SPEC §6
  `vertex_open_ot_loop_checkpoint` / `vertex_open_ot_signal_change` audit +
  resume layer (ADR-2606280030, ADR-2605151200 §6).

  SUBSTRATE BOUNDARY (CLAUDE.md): the Python original used SQLAlchemy Core
  over sqlite as a stand-in for RisingWave. RisingWave / Postgres is a
  PROHIBITED canonical-state backend in this repo — state belongs on the
  kotoba Datom log. So the persistence backend is modelled as an **injectable
  store seam** (`CheckpointStore` protocol). The default `InMemoryStore` is a
  pure-clj append log; a production impl backs the same protocol with
  `kt/transact` over the kotoba Datom log (no RisingWave, no SQLAlchemy).

  The serialization contract is preserved byte-identical with the Python:
    - internals dict → JSON of {did → base64(bytes)}
    - params_rev    → sha256 hex over sorted `did ':' params ';'`
    - checkpoint rows keyed by (loop-did, super-step)

  bytevec = vector of unsigned-byte ints 0..255."
  (:require [cheshire.core :as json])
  (:import [java.security MessageDigest]
           [java.util Base64]
           [java.time Instant]))

;; ---------------------------------------------------------------------------
;; byte / hash / base64 helpers (JVM/babashka)
;; ---------------------------------------------------------------------------

(defn- ->byte-array* ^bytes [bytevec]
  (byte-array (map (fn [b] (unchecked-byte (int b))) bytevec)))

(defn- byte-array-> [^bytes ba]
  (mapv (fn [b] (bit-and (int b) 0xFF)) ba))

(defn- b64-encode [bytevec]
  (.encodeToString (Base64/getEncoder) (->byte-array* bytevec)))

(defn- b64-decode [^String s]
  (byte-array-> (.decode (Base64/getDecoder) s)))

(defn- sha256-hex [^bytes ba]
  (let [md (MessageDigest/getInstance "SHA-256")]
    (->> (.digest md ba)
         (map #(format "%02x" (bit-and (int %) 0xFF)))
         (apply str))))

(defn- utc-iso-now [] (str (Instant/now)))

;; ---------------------------------------------------------------------------
;; serialization (port of _internals_to_json / _internals_from_json / _params_rev)
;; ---------------------------------------------------------------------------

(defn internals->json [internals]
  (json/generate-string
    (reduce-kv (fn [m did bytevec] (assoc m did (b64-encode bytevec))) {} internals)))

(defn internals<-json [s]
  (reduce-kv (fn [m did b64] (assoc m did (b64-decode b64)))
             {} (json/parse-string s)))

(defn params-rev
  "Stable sha256 hex of all cell params, for resume-validity checking.
  `cells` is the runner's did → cell-spec map (each holds `:params-bytes`)."
  [cells]
  (let [buf (java.io.ByteArrayOutputStream.)]
    (doseq [did (sort (keys cells))]
      (.write buf (.getBytes ^String did "UTF-8"))
      (.write buf (int (byte \:)))
      (.write buf (->byte-array* (:params-bytes (get cells did))))
      (.write buf (int (byte \;))))
    (sha256-hex (.toByteArray buf))))

;; ---------------------------------------------------------------------------
;; store seam — the kotoba-Datom-log boundary (default: in-memory)
;; ---------------------------------------------------------------------------

(defprotocol CheckpointStore
  (write-checkpoint! [store loop-did cp params-rev in-flight-msgs])
  (record-signal-change! [store signal-did value-micro-unit quality loop-dids-affected])
  (latest-checkpoint [store loop-did])
  (list-checkpoints [store loop-did])
  (count-checkpoints [store loop-did]))

;; checkpoint-row {:loop-did :super-step :ts :ecc-states :internals
;;                 :in-flight-msgs :params-rev}

(defrecord InMemoryStore [rows signals]
  CheckpointStore
  (write-checkpoint! [_ loop-did cp prev in-flight-msgs]
    (swap! rows conj
           {:loop-did       loop-did
            :super-step     (:super-step cp)
            :ts             (utc-iso-now)
            :ecc-states-json (json/generate-string (:ecc-states cp))
            :internals-json (internals->json (:internals cp))
            :in-flight-msgs-json (json/generate-string (or in-flight-msgs []))
            :params-rev     prev})
    nil)
  (record-signal-change! [_ signal-did value-micro-unit quality loop-dids-affected]
    (let [id (inc (count @signals))]
      (swap! signals conj
             {:change-id id
              :signal-did signal-did
              :ts (utc-iso-now)
              :value-micro-unit value-micro-unit
              :quality (or quality "good")
              :loop-did-affected-json (json/generate-string (or loop-dids-affected []))})
      id))
  (latest-checkpoint [_ loop-did]
    (when-let [r (->> @rows
                      (filter #(= (:loop-did %) loop-did))
                      (sort-by :super-step >)
                      first)]
      {:loop-did (:loop-did r)
       :super-step (:super-step r)
       :ts (:ts r)
       :ecc-states (json/parse-string (:ecc-states-json r))
       :internals (internals<-json (:internals-json r))
       :in-flight-msgs (json/parse-string (:in-flight-msgs-json r))
       :params-rev (:params-rev r)}))
  (list-checkpoints [_ loop-did]
    (->> @rows
         (filter #(= (:loop-did %) loop-did))
         (sort-by :super-step <)
         (mapv (fn [r]
                 {:loop-did (:loop-did r)
                  :super-step (:super-step r)
                  :ts (:ts r)
                  :ecc-states (json/parse-string (:ecc-states-json r))
                  :internals (internals<-json (:internals-json r))
                  :in-flight-msgs (json/parse-string (:in-flight-msgs-json r))
                  :params-rev (:params-rev r)}))))
  (count-checkpoints [_ loop-did]
    (if (nil? loop-did)
      (count @rows)
      (count (filter #(= (:loop-did %) loop-did) @rows)))))

(defn in-memory-store
  "Construct the default in-memory CheckpointStore (the kotoba-Datom-log seam's
  test/dev backend; production swaps in a kt/transact-backed store)."
  []
  (->InMemoryStore (atom []) (atom [])))

;; convenience arities mirroring the Python optional args
(defn write-cp!
  ([store loop-did cp prev] (write-checkpoint! store loop-did cp prev nil))
  ([store loop-did cp prev in-flight] (write-checkpoint! store loop-did cp prev in-flight)))

(defn record-signal!
  ([store signal-did value-micro-unit]
   (record-signal-change! store signal-did value-micro-unit "good" nil))
  ([store signal-did value-micro-unit quality]
   (record-signal-change! store signal-did value-micro-unit quality nil))
  ([store signal-did value-micro-unit quality loop-dids]
   (record-signal-change! store signal-did value-micro-unit quality loop-dids)))

;; ---------------------------------------------------------------------------
;; runner integration helpers (port of write_runner_checkpoint /
;; restore_runner_from_checkpointer)
;; ---------------------------------------------------------------------------

(defn write-runner-checkpoint!
  "Compute params-rev from the runner's cells + write the checkpoint."
  [store loop-did runner-cells cp]
  (write-checkpoint! store loop-did cp (params-rev runner-cells) nil))

(defn restore-runner-from-checkpointer
  "Load the latest checkpoint for `loop-did`, validate params-rev against the
  current runner cells, and return a pregel checkpoint map (or nil if absent).
  Throws `ex-info` on params-rev mismatch (would replay a different program).

  The actual `restore-from-checkpoint!` call is left to the caller so this ns
  stays free of a hard dep on pregel-runner."
  [store loop-did runner-cells]
  (when-let [row (latest-checkpoint store loop-did)]
    (let [expected (params-rev runner-cells)]
      (when (not= (:params-rev row) expected)
        (throw (ex-info (str "params_rev mismatch for " loop-did)
                        {:checkpoint (:params-rev row) :runner expected})))
      {:super-step (:super-step row)
       :ecc-states (:ecc-states row)
       :internals  (:internals row)
       :emissions  []})))
