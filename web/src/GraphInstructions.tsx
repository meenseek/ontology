/** The same gestures are described in the map and its detailed view settings. */
export default function GraphInstructions({ detailed = false, quick = false }: { detailed?: boolean; quick?: boolean }) {
  return <div className={`graph-instructions${quick ? " graph-quick-help" : ""}`}>
    <span className="pointer-instructions">빈 곳을 끌어 이동, <span className="zoom-instruction">스크롤 확대·축소,</span> 묶음을 눌러 펼치세요.</span>
    <span className="touch-instructions">빈 곳을 끌어 이동, <span className="zoom-instruction">두 손가락 확대·축소,</span> 묶음을 눌러 펼치세요.</span>
    {detailed && <> 별을 끌면 이동하며 묶음은 함께 움직입니다.</>}
  </div>;
}
