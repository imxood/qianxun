import AsyncStorage from '@react-native-async-storage/async-storage';
import type { Connection } from './types';

/** 与旧 Capacitor 壳同 schema，便于理解与人工核对。 */
const STORE_KEY = 'qx.connections.v1';

export async function loadConnections(): Promise<Connection[]> {
  try {
    const raw = await AsyncStorage.getItem(STORE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter(
      (item): item is Connection =>
        !!item &&
        typeof item === 'object' &&
        typeof (item as Connection).id === 'string' &&
        typeof (item as Connection).url === 'string' &&
        typeof (item as Connection).name === 'string',
    );
  } catch {
    return [];
  }
}

export async function saveConnections(
  connections: Connection[],
): Promise<void> {
  await AsyncStorage.setItem(STORE_KEY, JSON.stringify(connections));
}
