// The one bridge between the store and React.
//
// `useSyncExternalStore` rather than a context holding `useState`: the writers
// are the poll loop and the pickers, and the loop is not inside a component.
// One hook at the edge keeps that true as the port grows, instead of pushing
// state up into a provider that every slice then has to thread through.

import { useSyncExternalStore } from "react";

import { getState, subscribe, type ViewerState } from "./store.js";

export function useViewer(): ViewerState {
  return useSyncExternalStore(subscribe, getState, getState);
}
