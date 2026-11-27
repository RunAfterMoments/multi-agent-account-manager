// Cookie 一键复制的两段确认（MAAM-M-01）。
//
// 误触代价高：一次误点就把完整登录 Cookie 写进剪贴板。首次点击只进入确认态
// （按钮提示变「确认复制？」，3 秒内二次点击才真正复制，超时自动复原），不弹窗。
//
// 零运行时依赖（与 group-stream.ts 同一模式），以便 `node --test` 直接导入测试：
// render.ts 依赖 ./state 与 ./modals 的无后缀导入，Node 类型擦除加载器解析不了。
export const COOKIE_COPY_CONFIRM_MS = 3000;
export const COOKIE_COPY_LABEL = "复制 Cookie";
export const COOKIE_COPY_CONFIRM_LABEL = "确认复制？";

export interface CookieCopyConfirmHandle {
  /**
   * 处理一次点击：首次点击返回 false（仅进入确认态，不复制）；
   * 确认窗内再次点击返回 true（调用方执行复制）。
   */
  click(): boolean;
  /** 取消确认态并清理 timer；timer 回调也会先判 `isConnected` 再碰 DOM。 */
  dispose(): void;
}

/**
 * 状态存在按钮自身的 `data-copy-confirm` 上，不用模块级 map；timer 是按钮闭包
 * 内的单次 timeout。每次重渲染都会 `innerHTML` 重建按钮（见 main.ts `render()`），
 * 没有解绑钩子：旧按钮 detached 后 timer 回调靠 `isConnected` 守卫直接返回，
 * 不操作已不存在的 DOM，也不泄漏（单次 timeout，最长 3 秒即释放）。
 */
export function createCookieCopyConfirm(
  button: HTMLButtonElement,
  confirmMs: number = COOKIE_COPY_CONFIRM_MS,
): CookieCopyConfirmHandle {
  let timer: number | undefined;
  const clearTimer = (): void => {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  };
  const disarm = (): void => {
    clearTimer();
    if (!button.isConnected) return;
    delete button.dataset.copyConfirm;
    button.dataset.copyLabel = COOKIE_COPY_LABEL;
    button.classList.remove("confirming");
    button.setAttribute("aria-label", COOKIE_COPY_LABEL);
  };
  return {
    click(): boolean {
      if (button.dataset.copyConfirm === "1") {
        clearTimer();
        if (button.isConnected) {
          delete button.dataset.copyConfirm;
          button.classList.remove("confirming");
        }
        return true;
      }
      button.dataset.copyConfirm = "1";
      button.dataset.copyLabel = COOKIE_COPY_CONFIRM_LABEL;
      button.classList.add("confirming");
      button.classList.remove("copied");
      button.setAttribute("aria-label", "确认复制 Cookie？");
      clearTimer();
      timer = setTimeout(() => {
        timer = undefined;
        disarm();
      }, confirmMs);
      return false;
    },
    dispose: disarm,
  };
}
