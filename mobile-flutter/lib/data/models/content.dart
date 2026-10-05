import '../dsh/rpc_envelope.dart';

/// 内容块。一条消息的正文由若干块组成。
///
/// 线格式判别字段是 `type`（`ContentBlockSerializer`，手写序列化器）。
/// 未知类型**不丢**，落到 [ContentBlockUnknown] 原样保留——上游 harness 会
/// 增量扩展载荷，丢掉一个未知块等于丢掉半条消息。
sealed class ContentBlock {
  const ContentBlock();

  factory ContentBlock.fromJson(Map<String, Object?> json) {
    final type = json['type'] as String? ?? '';
    return switch (type) {
      'text' => ContentText((json['text'] as String?) ?? ''),
      'reasoning' => ContentReasoning((json['text'] as String?) ?? ''),
      'image' => ContentImage(ImageAttachment.fromJson(asMap(json['attachment']))),
      'file' => ContentFile(FileAttachment.fromJson(asMap(json['attachment']))),
      'tool-call' => ContentToolCall(
          id: (json['id'] as String?) ?? '',
          name: (json['name'] as String?) ?? '',
          // arguments 是**未解析的 JSON 字符串**，不是对象。保持原样透传。
          arguments: (json['arguments'] as String?) ?? '',
        ),
      'tool-result' => ContentToolResult(
          toolCallId: (json['toolCallId'] as String?) ?? '',
          content: _blocks(json['content']),
          isError: json['isError'] as bool?,
        ),
      _ => ContentBlockUnknown(type: type, raw: json),
    };
  }

  static List<ContentBlock> listFrom(Object? raw) => _blocks(raw);

  /// 该块的纯文本（供折叠/预览用）。
  String get plainText => switch (this) {
        ContentText(:final text) => text,
        ContentReasoning(:final text) => text,
        _ => '',
      };
}

/// 正常文本。
final class ContentText extends ContentBlock {
  const ContentText(this.text);
  final String text;
}

/// 思维链。与正文视觉上必须区分（次要色 + 可折叠），否则用户会把
/// 模型的内心戏当成结论。
final class ContentReasoning extends ContentBlock {
  const ContentReasoning(this.text);
  final String text;
}

/// 图片附件。
final class ContentImage extends ContentBlock {
  const ContentImage(this.attachment);
  final ImageAttachment attachment;
}

/// 文件附件。
final class ContentFile extends ContentBlock {
  const ContentFile(this.attachment);
  final FileAttachment attachment;
}

/// 工具调用。
final class ContentToolCall extends ContentBlock {
  const ContentToolCall({
    required this.id,
    required this.name,
    required this.arguments,
  });

  final String id;
  final String name;

  /// **未解析的 JSON 字符串**（线格式如此），不是 map。
  final String arguments;
}

/// 工具结果。
final class ContentToolResult extends ContentBlock {
  const ContentToolResult({
    required this.toolCallId,
    required this.content,
    this.isError,
  });

  final String toolCallId;
  final List<ContentBlock> content;
  final bool? isError;
}

/// 未知块，原样保留。
final class ContentBlockUnknown extends ContentBlock {
  const ContentBlockUnknown({required this.type, required this.raw});
  final String type;
  final Map<String, Object?> raw;
}

/// 图片附件引用。
class ImageAttachment {
  const ImageAttachment({
    required this.attachmentId,
    required this.mediaType,
    required this.bytes,
    required this.width,
    required this.height,
    this.name,
  });

  final String attachmentId;
  final String mediaType;
  final int bytes;
  final int width;
  final int height;
  final String? name;

  factory ImageAttachment.fromJson(Map<String, Object?> json) => ImageAttachment(
        attachmentId: (json['attachmentId'] as String?) ?? '',
        mediaType: (json['mediaType'] as String?) ?? 'image/png',
        bytes: (json['bytes'] as num?)?.toInt() ?? 0,
        width: (json['width'] as num?)?.toInt() ?? 0,
        height: (json['height'] as num?)?.toInt() ?? 0,
        name: json['name'] as String?,
      );

  /// 宽高比，供占位图保持正确形状。
  double get aspectRatio => (width <= 0 || height <= 0) ? 1.0 : width / height;
}

/// 文件附件引用。
class FileAttachment {
  const FileAttachment({
    required this.attachmentId,
    required this.name,
    required this.bytes,
  });

  final String attachmentId;
  final String name;
  final int bytes;

  factory FileAttachment.fromJson(Map<String, Object?> json) => FileAttachment(
        attachmentId: (json['attachmentId'] as String?) ?? '',
        name: (json['name'] as String?) ?? '未命名',
        bytes: (json['bytes'] as num?)?.toInt() ?? 0,
      );
}

/// 消息来源。
///
/// `kind` ∈ `user` / `plugin` / `model` / `tool`。它决定消息在转录里的
/// 视觉归属：用户消息右对齐，模型消息左对齐，工具消息由内容块自己表达。
class MessageSource {
  const MessageSource({
    required this.kind,
    this.plugin,
    this.provider,
    this.model,
    this.callId,
    this.requestId,
  });

  final String kind;
  final String? plugin;
  final String? provider;
  final String? model;
  final String? callId;
  final String? requestId;

  factory MessageSource.fromJson(Map<String, Object?> json) => MessageSource(
        kind: (json['kind'] as String?) ?? 'user',
        plugin: json['plugin'] as String?,
        provider: json['provider'] as String?,
        model: json['model'] as String?,
        callId: json['callId'] as String?,
        requestId: json['requestId'] as String?,
      );

  bool get isUser => kind == 'user';
  bool get isModel => kind == 'model';
  bool get isTool => kind == 'tool';
  bool get isPlugin => kind == 'plugin';
}

/// 一条消息。
class MessageData {
  const MessageData({
    required this.id,
    required this.role,
    required this.content,
    required this.source,
  });

  final String id;

  /// `system` / `user` / `assistant` / `tool`。
  final String role;
  final List<ContentBlock> content;
  final MessageSource source;

  factory MessageData.fromJson(Map<String, Object?> json) => MessageData(
        id: (json['id'] as String?) ?? '',
        role: (json['role'] as String?) ?? 'user',
        content: ContentBlock.listFrom(json['content']),
        source: MessageSource.fromJson(asMap(json['source'])),
      );

  MessageData copyWith({List<ContentBlock>? content}) => MessageData(
        id: id,
        role: role,
        content: content ?? this.content,
        source: source,
      );

  /// 拼出全部文本块（转录用）。
  String get plainText => content.whereType<ContentText>().map((b) => b.text).join('\n\n');

  /// 是否含思维链。
  bool get hasReasoning => content.any((b) => b is ContentReasoning);
}

/// 工具调用事件载荷。
class ToolCallData {
  const ToolCallData({
    required this.turn,
    required this.step,
    required this.callId,
    required this.name,
    required this.arguments,
  });

  final int turn;
  final int step;
  final String callId;
  final String name;

  /// 未解析 JSON 字符串。
  final String arguments;

  factory ToolCallData.fromJson(Map<String, Object?> json) => ToolCallData(
        turn: (json['turn'] as num?)?.toInt() ?? 0,
        step: (json['step'] as num?)?.toInt() ?? 0,
        callId: (json['callId'] as String?) ?? '',
        name: (json['name'] as String?) ?? '',
        arguments: (json['arguments'] as String?) ?? '',
      );
}

/// 工具结果引用（错误态）。
class ToolErrorRef {
  const ToolErrorRef({required this.name, required this.code});
  final String name;
  final String code;

  factory ToolErrorRef.fromJson(Map<String, Object?> json) => ToolErrorRef(
        name: (json['name'] as String?) ?? '',
        code: (json['code'] as String?) ?? '',
      );
}

/// 工具结果事件载荷。
class ToolResultData {
  const ToolResultData({
    required this.turn,
    required this.step,
    required this.message,
    this.error,
  });

  final int turn;
  final int step;
  final MessageData message;
  final ToolErrorRef? error;

  /// 解析 `tool/result` 事件。
  ///
  /// ⚠️ **这里必须同时读两处**。harness ≥0.1.7 的 format v4 把结果字段
  /// 从 `content[].type=="tool-result"` 提升到了**消息顶层**
  /// （`role:"tool"` + 顶层 `toolCallId`/`isError`/`content`），而 v3 仍把
  /// 它们留在 content 里的 tool-result 块中。只读一处会丢掉整个工具结果。
  factory ToolResultData.fromJson(Map<String, Object?> json) {
    final message = MessageData.fromJson(asMap(json['message']));
    final error = json['error'] == null ? null : ToolErrorRef.fromJson(asMap(json['error']));

    // v3：结果在 content 块里。
    ContentToolResult? v3;
    for (final block in message.content) {
      if (block is ContentToolResult) {
        v3 = block;
        break;
      }
    }
    // v4：结果在消息顶层。
    final v4 = json['toolCallId'] is String
        ? ContentToolResult(
            toolCallId: json['toolCallId'] as String,
            content: ContentBlock.listFrom(json['content']),
            isError: json['isError'] as bool?,
          )
        : null;

    final resolved = v4 ?? v3;
    return ToolResultData(
      turn: (json['turn'] as num?)?.toInt() ?? 0,
      step: (json['step'] as num?)?.toInt() ?? 0,
      message: resolved == null ? message : message.copyWith(content: [resolved]),
      error: error,
    );
  }
}

/// 助手消息事件载荷。
class AssistantMessageData {
  const AssistantMessageData({
    required this.turn,
    required this.step,
    required this.message,
    this.usage,
    this.interrupted,
  });

  final int turn;
  final int step;
  final MessageData message;
  final TokenUsage? usage;
  final bool? interrupted;

  factory AssistantMessageData.fromJson(Map<String, Object?> json) => AssistantMessageData(
        turn: (json['turn'] as num?)?.toInt() ?? 0,
        step: (json['step'] as num?)?.toInt() ?? 0,
        message: MessageData.fromJson(asMap(json['message'])),
        usage: json['usage'] == null ? null : TokenUsage.fromJson(asMap(json['usage'])),
        interrupted: json['interrupted'] as bool?,
      );
}

/// token 用量。
class TokenUsage {
  const TokenUsage({
    required this.inputTokens,
    required this.outputTokens,
    this.cacheReadTokens,
    this.cacheWriteTokens,
    this.reasoningTokens,
  });

  final int inputTokens;
  final int outputTokens;
  final int? cacheReadTokens;
  final int? cacheWriteTokens;
  final int? reasoningTokens;

  factory TokenUsage.fromJson(Map<String, Object?> json) => TokenUsage(
        inputTokens: (json['inputTokens'] as num?)?.toInt() ?? 0,
        outputTokens: (json['outputTokens'] as num?)?.toInt() ?? 0,
        cacheReadTokens: (json['cacheReadTokens'] as num?)?.toInt(),
        cacheWriteTokens: (json['cacheWriteTokens'] as num?)?.toInt(),
        reasoningTokens: (json['reasoningTokens'] as num?)?.toInt(),
      );

  /// 上下文占用 = 输入 + 缓存读 + 缓存写（不含输出，输出不占下一轮上下文）。
  int get contextTokens => inputTokens + (cacheReadTokens ?? 0) + (cacheWriteTokens ?? 0);
}

/// 待办条目。
class TodoItem {
  const TodoItem({required this.content, required this.status});
  final String content;

  /// `pending` / `in_progress` / `completed`。
  final String status;

  factory TodoItem.fromJson(Map<String, Object?> json) => TodoItem(
        content: (json['content'] as String?) ?? '',
        status: (json['status'] as String?) ?? 'pending',
      );

  bool get isDone => status == 'completed';
  bool get isActive => status == 'in_progress';
}

/// 后台任务。
///
/// `status` 的 `UNKNOWN` 分支很重要：harness 的状态联合是封闭的，但
/// 新增成员不该让整个任务列表解析失败——一个不认识的词不该吃掉整页数据。
enum JobStatus {
  running,
  stopping,
  completed,
  killed,
  failed,
  unknown;

  static JobStatus parse(String? raw) => switch (raw) {
        'running' => JobStatus.running,
        'stopping' => JobStatus.stopping,
        'completed' => JobStatus.completed,
        'killed' => JobStatus.killed,
        'failed' => JobStatus.failed,
        _ => JobStatus.unknown,
      };

  bool get isLive => this == JobStatus.running || this == JobStatus.stopping;
}

/// 一个后台任务（如 `bash`、`pwsh`、子 agent 委派）。
class JobView {
  const JobView({
    required this.id,
    required this.kind,
    required this.label,
    required this.status,
    required this.startedAt,
    this.detail,
    this.progress,
    this.finishedAt,
  });

  final String id;

  /// 生产者种类：`bash` / `pwsh` / `pty-send` / `subagent` / …
  final String kind;

  /// 一行标签：命令本身，或委派描述。
  final String label;
  final JobStatus status;

  /// 生产者给的细节，如 `exit code: 3`。
  final String? detail;

  /// 实时进度行，如 `3/10`（harness 0.1.7）。
  final String? progress;
  final int startedAt;
  final int? finishedAt;

  factory JobView.fromJson(Map<String, Object?> json) => JobView(
        id: (json['id'] as String?) ?? '',
        kind: (json['kind'] as String?) ?? '',
        label: (json['label'] as String?) ?? '',
        status: JobStatus.parse(json['status'] as String?),
        startedAt: (json['startedAt'] as num?)?.toInt() ?? 0,
        detail: json['detail'] as String?,
        progress: json['progress'] as String?,
        finishedAt: (json['finishedAt'] as num?)?.toInt(),
      );

  Duration? get elapsed {
    if (startedAt <= 0) return null;
    final end = finishedAt ?? DateTime.now().millisecondsSinceEpoch;
    final d = end - startedAt;
    return d < 0 ? null : Duration(milliseconds: d);
  }
}

/// 提问选项。
class QuestionOption {
  const QuestionOption({required this.label, this.description});
  final String label;
  final String? description;

  factory QuestionOption.fromJson(Map<String, Object?> json) => QuestionOption(
        label: (json['label'] as String?) ?? '',
        description: json['description'] as String?,
      );
}

/// 一个待答问题。
class QuestionItem {
  const QuestionItem({
    required this.id,
    required this.question,
    this.detail,
    this.header,
    this.options = const [],
    this.multiSelect,
    this.planReviewApprove,
  });

  final String id;
  final String question;
  final String? detail;
  final String? header;
  final List<QuestionOption> options;
  final bool? multiSelect;

  /// `intent.kind == "plan-review"` 时的批准选项标签。
  ///
  /// **协议建模，UI 消费推迟到 Batch 5。** 字段照留是因为它是服务端真实下发的
  /// 东西，删了将来还得重新探。`isPlanReview` 目前没有调用方：原先
  /// `_OptionRow` 用它给批准项单独上 `running` 绿，已按 05 §6.2
  /// 「状态色不进按钮填充」删掉。Batch 5 决定 plan-review 在待答链路里
  /// 到底怎么表达——大概率是给批准项加一个**可见中文标记**而不是靠颜色。
  final String? planReviewApprove;

  bool get isPlanReview => planReviewApprove != null;

  factory QuestionItem.fromJson(Map<String, Object?> json) {
    final intent = asMap(json['intent']);
    return QuestionItem(
      id: (json['id'] as String?) ?? '',
      question: (json['question'] as String?) ?? '',
      detail: json['detail'] as String?,
      header: json['header'] as String?,
      options: (json['options'] as List? ?? const [])
          .map((e) => QuestionOption.fromJson(asMap(e)))
          .toList(growable: false),
      multiSelect: json['multiSelect'] as bool?,
      planReviewApprove: intent['kind'] == 'plan-review' ? intent['approve'] as String? : null,
    );
  }
}

/// 一次提问的回答。
///
/// ⚠️ `custom` 挂在**每一条**上，不是挂在批次上。harness 的 zod 解析会
/// 静默剥掉放错位置的键，答案会变成空。
class QuestionAnswerItem {
  const QuestionAnswerItem({
    required this.id,
    this.selected = const [],
    this.custom,
  });

  final String id;

  /// 选中的标签原文，含任何 `(Recommended)` 后缀。
  final List<String> selected;

  /// 自由文本「其他」回答。空字符串视为**未填写**（会被拒）。
  final String? custom;

  Map<String, Object?> toJson() => {
        'id': id,
        'selected': selected,
        if (custom != null && custom!.trim().isNotEmpty) 'custom': custom,
      };
}

List<ContentBlock> _blocks(Object? raw) {
  if (raw is! List) return const [];
  return raw.map((e) => ContentBlock.fromJson(asMap(e))).toList(growable: false);
}
