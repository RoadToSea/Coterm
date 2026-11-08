import { useCallback, useEffect, useReducer } from "react";
import type { InstanceInfo } from "./host-api";

interface Views {
  ids: string[];
  activeId: string | null;
  driverId: string | null;
  initialized: boolean;
}

type Action = { type: "sync"; ids: string[] }
  | { type: "open"; id: string }
  | { type: "close"; id: string }
  | { type: "configure"; driverId: string | null };

function retainViews(state: Views, ids: string[]): Views {
  if (state.activeId && ids.includes(state.activeId)) return { ...state, ids };
  const index = state.activeId ? state.ids.indexOf(state.activeId) : 0;
  return { ...state, ids, activeId: ids[Math.min(Math.max(index, 0), ids.length - 1)] ?? null };
}

function reducer(state: Views, action: Action): Views {
  switch (action.type) {
    case "sync": {
      if (!state.initialized) return { ...state, ids: action.ids, activeId: action.ids[0] ?? null, initialized: true };
      const ids = state.ids.filter(id => action.ids.includes(id));
      return ids.length === state.ids.length ? state : retainViews(state, ids);
    }
    case "open": return { ...state, ids: state.ids.includes(action.id) ? state.ids : [...state.ids, action.id], activeId: action.id, driverId: null };
    case "close": return retainViews(state, state.ids.filter(id => id !== action.id));
    case "configure": return { ...state, driverId: action.driverId };
  }
}

/** View state is separate from backend instance lifetime; polling never reopens closed tabs. */
export function useInstanceViews(instances: InstanceInfo[], loading: boolean) {
  const [views, dispatch] = useReducer(reducer, { ids: [], activeId: null, driverId: null, initialized: false });
  useEffect(() => {
    if (!loading) dispatch({ type: "sync", ids: instances.map(instance => instance.driverInstanceId) });
  }, [instances, loading]);
  const open = useCallback((id: string) => dispatch({ type: "open", id }), []);
  const close = useCallback((id: string) => dispatch({ type: "close", id }), []);
  const configure = useCallback((driverId: string | null) => dispatch({ type: "configure", driverId }), []);
  return { ...views, open, close, configure };
}
