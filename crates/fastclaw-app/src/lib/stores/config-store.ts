import { create } from "zustand";

export interface ConfigStoreState {
  _placeholder?: undefined;
}

export const useConfigStore = create<ConfigStoreState>(() => ({}));
