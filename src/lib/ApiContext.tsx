import { createContext, useContext } from "react";
import type { Api } from "./api";

export const ApiContext = createContext<Api | null>(null);

export function useApi(): Api {
  const a = useContext(ApiContext);
  if (!a) throw new Error("API not ready");
  return a;
}
