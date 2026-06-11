import { useEffect } from "react";
import { IconContext } from "@phosphor-icons/react";
import { AppLayout } from "./components/layout/AppLayout";
import { ContextMenuProvider } from "./components/common/ContextMenu";
import { TooltipProvider } from "./components/common/Tooltip";
import { ImageLightbox } from "./components/common/ImageLightbox";
import { useGatewayStore } from "./lib/store";
import { ICON_SIZE } from "./lib/ui-tokens";
import "./lib/theme";

const iconContextValue = {
  size: ICON_SIZE.sm,
  weight: "regular" as const,
  color: "currentColor",
};

export default function App() {
  const initGateway = useGatewayStore((s) => s.init);

  useEffect(() => {
    initGateway();
  }, [initGateway]);

  return (
    <IconContext.Provider value={iconContextValue}>
      <AppLayout />
      <ContextMenuProvider />
      <TooltipProvider />
      <ImageLightbox />
    </IconContext.Provider>
  );
}
