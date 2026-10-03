import { useEffect, useId, useRef, useState, type ReactNode } from "react";

export type AttachmentMedia = { kind: "image" | "video"; url: string };
export function attachmentMediaKind(path: string): AttachmentMedia["kind"] | undefined {
  if (/\.(png|jpe?g|gif|webp)$/i.test(path)) return "image";
  if (/\.(mp4|m4v|webm)$/i.test(path)) return "video";
}

/** Mount media only on request; closing also stops playback and releases the element. */
export default function AttachmentPreview({ media, download, label, description }: { media: AttachmentMedia; download: string; label: ReactNode; description: string }) {
  const [open, setOpen] = useState(false), [failed, setFailed] = useState(false), [loaded, setLoaded] = useState(false);
  const id = useId();
  const player = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const video = player.current;
    return () => {
      // Explicitly stop playback and pending media loads, including navigation/unmount.
      if (video) { video.pause(); video.removeAttribute("src"); video.load(); }
    };
  }, [open, failed, media.url]);
  const noun = media.kind === "image" ? "사진" : "영상";
  const show = () => { setFailed(false); setLoaded(false); setOpen(true); };
  return <span className="attachment-preview">
    <span className="attachment-actions">
      <span>{label}</span>
      <button type="button" aria-expanded={open} aria-controls={id} onClick={() => open ? setOpen(false) : show()}>{noun} {open ? "닫기" : "미리보기"}</button>
      <a href={download} download>다운로드</a>
    </span>
    <span id={id} className="attachment-media" hidden={!open}>
      {open && (failed ? <span className="attachment-error" role="alert">{noun}을 열 수 없습니다. 원본 다운로드로 확인해 주세요. <button type="button" onClick={show}>다시 시도</button></span> : <>
        {media.kind === "image" ? <>
          {!loaded && <span className="hint" role="status">사진을 불러오는 중…</span>}
          <img src={media.url} alt={description} onLoad={() => setLoaded(true)} onError={() => setFailed(true)} />
        </> : <video ref={player} src={media.url} aria-label={description} controls playsInline preload="none" onError={() => setFailed(true)} />}
      </>)}
    </span>
  </span>;
}
