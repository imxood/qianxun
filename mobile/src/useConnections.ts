import { useCallback, useEffect, useRef, useState } from 'react';
import { probeInfo } from './probe';
import { loadConnections, saveConnections } from './store';
import type { Connection } from './types';
import { hostOf, isValidPairUrl, newId } from './validate';

export interface AddResult {
  ok: boolean;
  message: string;
}

export interface Notice {
  kind: 'ok' | 'error';
  text: string;
}

/** 连接集合：加载/持久化/增改 + 添加时的自动命名与在线识别。 */
export function useConnections() {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [ready, setReady] = useState(false);
  const connectionsRef = useRef(connections);
  connectionsRef.current = connections;

  useEffect(() => {
    loadConnections().then(list => {
      setConnections(list);
      setReady(true);
    });
  }, []);

  const persist = useCallback((next: Connection[]) => {
    setConnections(next);
    void saveConnections(next);
  }, []);

  const addConnection = useCallback(
    async (rawUrl: string, manualName: string): Promise<AddResult> => {
      const url = rawUrl.trim();
      if (!isValidPairUrl(url)) {
        return {
          ok: false,
          message:
            '链接形态不对：请扫电脑端「远程访问」的配对二维码，或完整粘贴配对链接。',
        };
      }
      if (connectionsRef.current.some(item => item.url === url)) {
        return { ok: false, message: '这条连接已经存在。' };
      }
      const draft: Connection = {
        id: newId(),
        name: manualName.trim() || hostOf(url),
        url,
        addedAt: Date.now(),
        lastUsedAt: 0,
      };
      const info = await probeInfo(draft).catch(() => null);
      if (info?.hostname && !manualName.trim()) {
        draft.name = info.hostname;
      }
      persist([...connectionsRef.current, draft]);
      if (info) {
        return {
          ok: true,
          message: `已添加「${draft.name}」${info.dshReady ? '，DSH 运行中' : '（DSH 未运行，稍后可连）'}`,
        };
      }
      return {
        ok: true,
        message:
          '暂时连不上这台电脑（可能离线），已按地址保存，连上后状态点亮。',
      };
    },
    [persist],
  );

  const renameConnection = useCallback(
    (id: string, name: string) => {
      const trimmed = name.trim();
      if (!trimmed) {
        return;
      }
      persist(
        connectionsRef.current.map(item =>
          item.id === id ? { ...item, name: trimmed } : item,
        ),
      );
    },
    [persist],
  );

  const updateConnectionUrl = useCallback(
    (id: string, url: string): AddResult => {
      if (!isValidPairUrl(url)) {
        return {
          ok: false,
          message: '链接形态不对：应来自千寻设置页的配对二维码/复制链接。',
        };
      }
      persist(
        connectionsRef.current.map(item =>
          item.id === id ? { ...item, url } : item,
        ),
      );
      return { ok: true, message: '配对链接已更新。' };
    },
    [persist],
  );

  const removeConnection = useCallback(
    (id: string) => {
      persist(connectionsRef.current.filter(item => item.id !== id));
    },
    [persist],
  );

  const touchConnection = useCallback(
    (id: string) => {
      persist(
        connectionsRef.current.map(item =>
          item.id === id ? { ...item, lastUsedAt: Date.now() } : item,
        ),
      );
    },
    [persist],
  );

  return {
    connections,
    ready,
    addConnection,
    renameConnection,
    updateConnectionUrl,
    removeConnection,
    touchConnection,
  };
}
