<script lang="ts">
	import {
		Plus,
		Search,
		ChevronDown,
		ChevronRight,
		ChevronUp,
		Folder,
		FolderOpen,
		FolderPlus,
		MessageSquare,
		MoreHorizontal,
		Zap,
		Settings,
		Sparkles,
		AtSign,
		Paperclip,
		Cpu,
		ArrowUp,
		ArrowLeft,
		ShieldCheck,
		Loader,
		Check,
		CheckCircle2,
		XCircle,
		X,
		AlertCircle,
		Send,
		Sun,
		Moon,
		LayoutGrid,
		Layers,
		Users,
		Archive,
		GitBranch,
		FileCode,
		FileText,
		FilePlus,
		GitPullRequest,
		Info,
		ExternalLink,
		RefreshCw,
		PanelRightClose,
		PanelRightOpen,
		Bug,
		Code2,
		FlaskConical,
		Hash,
		Clock,
		MessageSquareDashed,
		MessageSquarePlus,
	} from '@lucide/svelte';

	type IconName =
		| 'plus'
		| 'plus-circle'
		| 'plus-square'
		| 'search'
		| 'chevron-down'
		| 'chevron-right'
		| 'chevron-up'
		| 'folder'
		| 'folder-open'
		| 'folder-plus'
		| 'archive'
		| 'message-square'
		| 'message-square-dashed'
		| 'message-square-plus'
		| 'message-square-text'
		| 'more-horizontal'
		| 'zap'
		| 'settings'
		| 'sparkles'
		| 'at-sign'
		| 'paperclip'
		| 'cpu'
		| 'arrow-up'
		| 'arrow-left'
		| 'shield-check'
		| 'loader'
		| 'check'
		| 'check-circle-2'
		| 'x-circle'
		| 'x'
		| 'alert-circle'
		| 'send'
		| 'sun'
		| 'moon'
		| 'layout-grid'
		| 'layers'
		| 'users'
		| 'git-branch'
		| 'file-code'
		| 'file-text'
		| 'file-plus'
		| 'git-pull-request'
		| 'info'
		| 'external-link'
		| 'refresh-cw'
		| 'panel-right-close'
		| 'panel-right-open'
		| 'bug'
		| 'code-2'
		| 'flask-conical'
		| 'hash'
		| 'clock';

	let { name, class: className = 'w-4 h-4', strokeWidth = 2 }: { name: IconName; class?: string; strokeWidth?: number } = $props();

	const iconMap = {
		plus: Plus,
		'plus-circle': Plus,
		'plus-square': Plus,
		search: Search,
		'chevron-down': ChevronDown,
		'chevron-right': ChevronRight,
		'chevron-up': ChevronUp,
		folder: Folder,
		'folder-open': FolderOpen,
		'folder-plus': FolderPlus,
		archive: Archive,
		'message-square': MessageSquare,
		'message-square-dashed': MessageSquareDashed,
		'message-square-plus': MessageSquarePlus,
		'message-square-text': MessageSquare,
		'more-horizontal': MoreHorizontal,
		zap: Zap,
		settings: Settings,
		sparkles: Sparkles,
		'at-sign': AtSign,
		paperclip: Paperclip,
		cpu: Cpu,
		'arrow-up': ArrowUp,
		'arrow-left': ArrowLeft,
		'shield-check': ShieldCheck,
		loader: Loader,
		check: Check,
		'check-circle-2': CheckCircle2,
		'x-circle': XCircle,
		x: X,
		'alert-circle': AlertCircle,
		send: Send,
		sun: Sun,
		moon: Moon,
		'layout-grid': LayoutGrid,
		layers: Layers,
		users: Users,
		'git-branch': GitBranch,
		'file-code': FileCode,
		'file-text': FileText,
		'file-plus': FilePlus,
		'git-pull-request': GitPullRequest,
		info: Info,
		'external-link': ExternalLink,
		'refresh-cw': RefreshCw,
		'panel-right-close': PanelRightClose,
		'panel-right-open': PanelRightOpen,
		bug: Bug,
		'code-2': Code2,
		'flask-conical': FlaskConical,
		hash: Hash,
		clock: Clock,
	};
	const Selected = $derived(iconMap[name] || Plus);
</script>

<Selected class={className} {strokeWidth} />
