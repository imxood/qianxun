/** 概览页的路线图数据。v0.1 交付 + v0.2 功能深化（设计 docs/05）。 */
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
    summary: 'Alt+A 全局快捷键、六种标注、复制/保存/贴图 Pin',
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
    title: '同步（一阶段）',
    summary: '笔记库 git 化 + DSH 配置 JSON 镜像（ADR-013）',
    done: true,
  },
  {
    version: 'v0.1',
    title: '远程骨架 / Android 骨架',
    summary: '网关与配对 UI 骨架、Capacitor 壳骨架——均未达可用（生效链路与出包在 v0.2 完成）',
    done: false,
  },
  {
    version: 'v0.2',
    title: '可用性修复（P0）',
    summary: '终端首会话渲染门与输出回放、笔记编辑器游离视图与元数据契约、远程配置指纹即时生效',
    done: true,
  },
  {
    version: 'v0.2',
    title: '搜索重构：工作区 Everything（P1）',
    summary: '盘符根选择、可排序结果表、右键菜单、键盘流、流式实时 grep、glob 过滤',
    done: true,
  },
  {
    version: 'v0.2',
    title: '截屏交互（P1）',
    summary: '删「选择」按钮：框完即可拖，边带/手柄任何工具态可用',
    done: true,
  },
  {
    version: 'v0.2',
    title: '远程闭环（P2）',
    summary: '一级「远程」页：网关配置/配对/设备/自检；配对吊销即时生效 + 集成测试',
    done: true,
  },
  {
    version: 'v0.2',
    title: 'Android 出包（P3）',
    summary: 'JDK21/SDK 装到 D:\\programs、壳工程脚本化配置、debug APK',
    done: true,
  },
];
