/** 概览页的路线图数据。全部里程碑均在 v0.1 交付。 */
export interface RoadmapItem {
  version: string;
  title: string;
  summary: string;
  /** 已交付的里程碑打勾。 */
  done?: boolean;
}

export const roadmap: RoadmapItem[] = [
  {
    version: 'v0.1',
    title: '核心托管',
    summary: 'Node/DSH 检测与一键安装、固定端口启动、健康守护、iframe 承载',
    done: true,
  },
  {
    version: 'v0.1',
    title: '搜索双页',
    summary: '文件名搜索（fff-search 三模式）+ 内容搜索（fff-grep，上下文/进度/取消）',
    done: true,
  },
  {
    version: 'v0.1',
    title: '微信级截屏',
    summary: 'Alt+A 全局快捷键、矩形/椭圆/箭头/画笔/马赛克/文字、复制/保存/贴图 Pin',
    done: true,
  },
  {
    version: 'v0.1',
    title: '多标签终端',
    summary: 'portable-pty + xterm 6 GPU 渲染，切标签不丢会话',
    done: true,
  },
  {
    version: 'v0.1',
    title: '笔记',
    summary: 'Markdown 笔记库（纯本地文件夹）、编辑器与检索',
    done: true,
  },
  {
    version: 'v0.1',
    title: 'DSH 插件桥',
    summary: '笔记 AI 整理（llm.stream）、笔记注册为 agent 工具',
    done: true,
  },
  {
    version: 'v0.1',
    title: 'EasyTier 远程',
    summary: '网关绑定虚拟网卡、扫码配对、每设备密钥',
    done: true,
  },
  {
    version: 'v0.1',
    title: 'Android',
    summary: 'Capacitor 壳加载网关地址，移动办公（骨架）',
    done: true,
  },
  {
    version: 'v0.1',
    title: '同步',
    summary: '笔记库与 DSH 配置的文件级同步（一阶段）',
    done: true,
  },
];
