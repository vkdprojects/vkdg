import { writable } from 'svelte/store';
import type { SessionUser } from './api.js';

export const user = writable<SessionUser | null | undefined>(undefined); // undefined = loading
