import { useEffect, useRef, useState } from "react";
import type { Api } from "./api";
import type { Filters, SearchResults } from "./types";

/** Search-as-you-type with debounce; stale responses are dropped. */
export function useSearch(api: Api, text: string, filters: Filters, limit: number, refreshKey = 0) {
  const [results, setResults] = useState<SearchResults | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const seq = useRef(0);
  const key = JSON.stringify(filters);
  useEffect(() => {
    const my = ++seq.current;
    setLoading(true);
    const delay = text.trim() ? 110 : 0;
    const t = setTimeout(() => {
      api
        .search({ text, filters, limit })
        .then((r) => {
          if (my === seq.current) {
            setResults(r);
            setError(null);
          }
        })
        .catch((e) => my === seq.current && setError(String(e)))
        .finally(() => my === seq.current && setLoading(false));
    }, delay);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, text, key, limit, refreshKey]);
  return { results, error, loading };
}
