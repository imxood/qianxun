import { hostOf, isValidPairUrl, relTime } from '../src/validate';

describe('配对链接校验', () => {
  it('接受千寻配对链接形态', () => {
    expect(
      isValidPairUrl('http://10.144.3.2:17400/qx-gate?token=cafebabe12345678'),
    ).toBe(true);
    expect(
      isValidPairUrl('  http://10.144.3.2:17400/qx-gate?token=CAFEF00D  '),
    ).toBe(true);
    expect(
      isValidPairUrl('http://my-pc.local:17400/qx-gate?token=deadbeef'),
    ).toBe(true);
  });

  it('拒绝非配对形态', () => {
    expect(isValidPairUrl('http://10.144.3.2:17400/')).toBe(false);
    expect(
      isValidPairUrl('https://10.144.3.2:17400/qx-gate?token=cafebabe'),
    ).toBe(false);
    expect(isValidPairUrl('http://10.144.3.2:17400/qx-gate?token=短')).toBe(
      false,
    );
    expect(isValidPairUrl('随便一段文本')).toBe(false);
    expect(isValidPairUrl('')).toBe(false);
  });
});

describe('展示工具', () => {
  it('hostOf 提取主机端口', () => {
    expect(hostOf('http://10.144.3.2:17400/qx-gate?token=x')).toBe(
      '10.144.3.2:17400',
    );
    expect(hostOf('not a url')).toBe('not a url');
  });

  it('relTime 相对时间', () => {
    expect(relTime(0)).toBe('从未使用');
    expect(relTime(Date.now() - 10_000)).toBe('刚刚使用');
    expect(relTime(Date.now() - 5 * 60_000)).toBe('5 分钟前使用');
    expect(relTime(Date.now() - 3 * 60 * 60_000)).toBe('3 小时前使用');
    expect(relTime(Date.now() - 2 * 24 * 60 * 60_000)).toBe('2 天前使用');
  });
});
