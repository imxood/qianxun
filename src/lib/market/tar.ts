/**
 * 从 gzip 压缩的 tar 包里抽一个文本文件（npm tarball 的形态）。
 *
 * tar 以 512 字节块为单位：文件名 @0（100 字节）、八进制大小 @124（12 字节）、
 * 类型 @156；'0' 与 NUL 都是普通文件，其余（目录/pax 头）跳过。
 * 解压用 WebView2 原生的 DecompressionStream，零依赖。
 */

const BLOCK = 512;

export async function fileFromTarGz(gz: ArrayBuffer, wanted: string): Promise<string | null> {
  const stream = new Blob([gz]).stream().pipeThrough(new DecompressionStream('gzip'));
  const buf = await new Response(stream).arrayBuffer();
  const view = new DataView(buf);
  const decode = new TextDecoder();
  let offset = 0;
  while (offset + BLOCK <= buf.byteLength) {
    const name = decode.decode(new Uint8Array(buf, offset, 100)).replace(/\0.*$/u, '');
    // 连续两个空块收尾 tar；一个即够停。
    if (name === '') break;
    const rawSize = decode
      .decode(new Uint8Array(buf, offset + 124, 12))
      .replace(/\0.*$/u, '')
      .trim();
    const size = Number.parseInt(rawSize, 8);
    if (!Number.isFinite(size) || size < 0) break;
    const type = String.fromCharCode(view.getUint8(offset + 156));
    offset += BLOCK;
    if ((type === '0' || type === '\0') && name === wanted) {
      return decode.decode(new Uint8Array(buf, offset, size));
    }
    offset += Math.ceil(size / BLOCK) * BLOCK;
  }
  return null;
}
