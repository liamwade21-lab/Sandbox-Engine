// Sandbox Engine patch b (vendor/ruffle/UPSTREAM.md): Scaleform GFx 4's InteractiveObjectEx. Both
// settings concern mouse picking and drawing order of the mouse's topmost level; this player takes no
// mouse input, so they have no effect.
package scaleform.gfx {
    import flash.display.InteractiveObject;

    public class InteractiveObjectEx extends DisplayObjectEx {
        public static function setHitTestDisable(object:InteractiveObject, value:Boolean):void {
        }

        public static function getHitTestDisable(object:InteractiveObject):Boolean {
            return false;
        }

        public static function setTopmostLevel(object:InteractiveObject, value:Boolean):void {
        }

        public static function getTopmostLevel(object:InteractiveObject):Boolean {
            return false;
        }
    }
}
