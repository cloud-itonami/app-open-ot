(ns open-ot-orchestrator.droop-codec
  "Idiomatic clj/cljc port of `_generated/droop_p_f.py` (the codegen'd
  DROOP_P_F struct codec) per ADR-2606280030.

  Faithful little-endian byte (un)packing of the DROOP_P_F cell wire structs.
  The Python original used `struct.pack`/`struct.unpack`; this implements the
  same layouts with pure arithmetic over a vector of unsigned-byte ints
  (0..255), so it loads + runs under babashka with NO host interop and NO
  external dependency.

  FBType   : DROOP_P_F
  Layouts (verified byte-identical against the Python `struct` formats):
    DataIn   '<qqiBB2x' size 24
    DataOut  '<iiqBB6x' size 24
    Params   '<iiiiiI'  size 24
    Internal '<iB3x'    size 8

  NOTE: this is the substrate-agnostic *engineering layer* only. The actual
  cell math runs inside the Rust→WASM `droop_p_f.wasm` artefact loaded by
  `cell_loader.py` (wasmtime, no babashka binding — kept as `.py`)."
  (:require [clojure.string :as str]))

;; ---------------------------------------------------------------------------
;; little-endian integer codec over a byte vector (unsigned ints 0..255)
;; ---------------------------------------------------------------------------

(defn- le-bytes
  "Encode integer `v` as `width` little-endian unsigned bytes (two's-complement
  for negatives). Returns a vector of ints 0..255."
  [v width]
  (mapv (fn [i] (bit-and (bit-shift-right (long v) (* 8 i)) 0xFF))
        (range width)))

(defn- le->unsigned
  "Decode `width` little-endian bytes at `off` of `bs` into an unsigned long."
  [bs off width]
  (reduce (fn [acc i]
            (bit-or acc (bit-shift-left (long (nth bs (+ off i))) (* 8 i))))
          0
          (range width)))

(defn- le->i32 [bs off]
  (let [u (le->unsigned bs off 4)]
    (if (>= u 0x80000000) (- u 0x100000000) u)))

(defn- le->u32 [bs off] (le->unsigned bs off 4))

(defn- le->i64 [bs off]
  ;; Assembling into a long already yields the correct two's-complement value
  ;; because the high byte's sign bit lands on bit 63.
  (le->unsigned bs off 8))

(defn- put!
  "Write `bytes-vec` starting at `off` into transient vector `tv`."
  [tv off bytes-vec]
  (reduce (fn [t i] (assoc! t (+ off i) (nth bytes-vec i)))
          tv
          (range (count bytes-vec))))

;; ---------------------------------------------------------------------------
;; DataIn  '<qqiBB2x' size 24
;; ---------------------------------------------------------------------------

(def data-in-size 24)

(defn pack-data-in
  "{:grid-freq i64 :freq-nominal i64 :current-p i32 :freq-quality u8 :enable bool}
   → vector of 24 unsigned bytes."
  [{:keys [grid-freq freq-nominal current-p freq-quality enable]}]
  (persistent!
    (-> (transient (vec (repeat data-in-size 0)))
        (put! 0  (le-bytes grid-freq 8))
        (put! 8  (le-bytes freq-nominal 8))
        (put! 16 (le-bytes current-p 4))
        (put! 20 (le-bytes (or freq-quality 0) 1))
        (put! 21 (le-bytes (if enable 1 0) 1)))))

(defn unpack-data-in [bs]
  {:grid-freq    (le->i64 bs 0)
   :freq-nominal (le->i64 bs 8)
   :current-p    (le->i32 bs 16)
   :freq-quality (nth bs 20)
   :enable       (not (zero? (nth bs 21)))})

;; ---------------------------------------------------------------------------
;; DataOut '<iiqBB6x' size 24
;; ---------------------------------------------------------------------------

(def data-out-size 24)

(defn pack-data-out
  "{:p-setpoint i32 :delta-p i32 :freq-error i64 :dead-band-active bool :saturated bool}"
  [{:keys [p-setpoint delta-p freq-error dead-band-active saturated]}]
  (persistent!
    (-> (transient (vec (repeat data-out-size 0)))
        (put! 0  (le-bytes p-setpoint 4))
        (put! 4  (le-bytes delta-p 4))
        (put! 8  (le-bytes freq-error 8))
        (put! 16 (le-bytes (if dead-band-active 1 0) 1))
        (put! 17 (le-bytes (if saturated 1 0) 1)))))

(defn unpack-data-out [bs]
  {:p-setpoint       (le->i32 bs 0)
   :delta-p          (le->i32 bs 4)
   :freq-error       (le->i64 bs 8)
   :dead-band-active (not (zero? (nth bs 16)))
   :saturated        (not (zero? (nth bs 17)))})

;; ---------------------------------------------------------------------------
;; Params '<iiiiiI' size 24
;; ---------------------------------------------------------------------------

(def params-size 24)

(defn pack-params
  "{:p-rated-micro-kw i32 :p-min-micro-kw i32 :p-max-micro-kw i32
    :droop-permille i32 :dead-band-micro-hz i32 :cycle-period-ms u32}"
  [{:keys [p-rated-micro-kw p-min-micro-kw p-max-micro-kw
           droop-permille dead-band-micro-hz cycle-period-ms]}]
  (persistent!
    (-> (transient (vec (repeat params-size 0)))
        (put! 0  (le-bytes p-rated-micro-kw 4))
        (put! 4  (le-bytes p-min-micro-kw 4))
        (put! 8  (le-bytes p-max-micro-kw 4))
        (put! 12 (le-bytes droop-permille 4))
        (put! 16 (le-bytes dead-band-micro-hz 4))
        (put! 20 (le-bytes cycle-period-ms 4)))))

(defn unpack-params [bs]
  {:p-rated-micro-kw   (le->i32 bs 0)
   :p-min-micro-kw     (le->i32 bs 4)
   :p-max-micro-kw     (le->i32 bs 8)
   :droop-permille     (le->i32 bs 12)
   :dead-band-micro-hz (le->i32 bs 16)
   :cycle-period-ms    (le->u32 bs 20)})

;; ---------------------------------------------------------------------------
;; Internal '<iB3x' size 8
;; ---------------------------------------------------------------------------

(def internal-size 8)

(defn pack-internal
  "{:last-setpoint-micro-kw i32 :initialized bool}"
  [{:keys [last-setpoint-micro-kw initialized]}]
  (persistent!
    (-> (transient (vec (repeat internal-size 0)))
        (put! 0 (le-bytes last-setpoint-micro-kw 4))
        (put! 4 (le-bytes (if initialized 1 0) 1)))))

(defn unpack-internal [bs]
  {:last-setpoint-micro-kw (le->i32 bs 0)
   :initialized            (not (zero? (nth bs 4)))})

;; ---------------------------------------------------------------------------
;; engineering-unit helpers (port of microgrid_pregel.py wrappers)
;; ---------------------------------------------------------------------------

(defn pack-droop-params
  "User-facing engineering units → packed Params bytes (µ-units inside)."
  [{:keys [p-rated-kw p-min-kw p-max-kw droop-pct dead-band-hz cycle-period-ms]}]
  (pack-params
    {:p-rated-micro-kw   (long (* p-rated-kw 1000000))
     :p-min-micro-kw     (long (* p-min-kw 1000000))
     :p-max-micro-kw     (long (* p-max-kw 1000000))
     :droop-permille     (long (* droop-pct 10))
     :dead-band-micro-hz (long (* dead-band-hz 1000000))
     :cycle-period-ms    (long cycle-period-ms)}))

(defn pack-droop-data-in
  [{:keys [grid-freq-hz freq-nominal-hz current-p-kw enable freq-quality]
    :or   {enable true freq-quality 0}}]
  (pack-data-in
    {:grid-freq    (long (* grid-freq-hz 1000000))
     :freq-nominal (long (* freq-nominal-hz 1000000))
     :current-p    (long (* current-p-kw 1000000))
     :freq-quality freq-quality
     :enable       enable}))

(defn unpack-droop-data-out
  "Packed DataOut bytes → engineering units (kW, Hz)."
  [bs]
  (let [raw (unpack-data-out bs)]
    {:p-setpoint-kw    (/ (:p-setpoint raw) 1000000.0)
     :delta-p-kw       (/ (:delta-p raw) 1000000.0)
     :freq-error-hz    (/ (:freq-error raw) 1000000.0)
     :dead-band-active (:dead-band-active raw)
     :saturated        (:saturated raw)}))

(defn hex
  "Lowercase hex string of a byte vector (debug / parity helper)."
  [bs]
  (str/join (map #(format "%02x" (int %)) bs)))
