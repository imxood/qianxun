/** 一条已保存的电脑连接（qx.connections.v1）。 */
export interface Connection {
  id: string;
  name: string;
  /** 千寻配对链接：http://<EasyTier-IP>:17400/qx-gate?token=… */
  url: string;
  addedAt: number;
  lastUsedAt: number;
}

/** 网关 /qx-mobile/info 返回的电脑端身份。 */
export interface PcInfo {
  app?: string;
  version?: string;
  hostname?: string;
  dshReady?: boolean;
}
