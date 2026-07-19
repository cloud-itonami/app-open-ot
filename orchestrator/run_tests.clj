#!/usr/bin/env bb
;; Test runner for the open-ot orchestrator clj/cljc port (ADR-2606280030).
;; Repo rule: clj runner, NOT a .sh. Run with `bb run_tests.clj` or `bb test`.
(require '[clojure.test :as t]
         'open-ot-orchestrator.core-test)

(let [{:keys [fail error]} (t/run-tests 'open-ot-orchestrator.core-test)]
  (System/exit (if (pos? (+ (or fail 0) (or error 0))) 1 0)))
