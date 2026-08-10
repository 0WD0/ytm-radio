(() => {
  const get = (name) => {
    try {
      if (globalThis.ytcfg && typeof globalThis.ytcfg.get === "function") {
        const value = globalThis.ytcfg.get(name);
        if (value !== undefined) return value;
      }
      if (globalThis.ytcfg && globalThis.ytcfg.data_) {
        const value = globalThis.ytcfg.data_[name];
        if (value !== undefined) return value;
      }
    } catch (_) {}
    return null;
  };
  const text = (value) => {
    if (value == null) return null;
    const result = String(value).trim();
    return result || null;
  };
  const context = get("INNERTUBE_CONTEXT");
  const sessionIndex = text(get("SESSION_INDEX"));
  const delegatedSessionId = text(get("DELEGATED_SESSION_ID"));
  const dataSyncId = text(get("DATASYNC_ID"));
  const contextPageId = text(
    context && context.user && context.user.onBehalfOfUser
  );
  if (!sessionIndex && !delegatedSessionId && !dataSyncId && !contextPageId) {
    return null;
  }
  return {
    innertubeContext: context || null,
    sessionIndex,
    delegatedSessionId,
    dataSyncId,
    userAgent: navigator.userAgent || null
  };
})()
