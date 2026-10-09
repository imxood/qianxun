// qrcode 的核心编码器没有官方类型声明（@types/qrcode 只覆盖顶层 API），
// 这里只声明我们真正用到的一个函数，避免 `any` 渗进渲染层。
declare module 'qrcode/lib/core/qrcode' {
  interface QrCode {
    modules: { size: number; data: Uint8Array };
  }
  interface CreateOptions {
    errorCorrectionLevel?: 'L' | 'M' | 'Q' | 'H';
    version?: number;
  }
  export function create(text: string, options?: CreateOptions): QrCode;
}
