package elitegfx {
    import flash.external.ExternalInterface;
    import flash.utils.getDefinitionByName;

    // The bridge between Elite's movies and the game (ARCHITECTURE.md A10). Its code block is put
    // before the movie's own (src/convert.rs) and its script runs when the movie's first frame
    // loads: it registers the game's callbacks. Elite's movies take their Scaleform path
    // (scaleform.gfx.Extensions.isScaleform is true, Ruffle patch b): UIApplication calls
    // ExternalInterface.call("BindDirectInterfaces"), and as Elite's exe answers it by filling in the
    // movie's factories with its native objects, the game hands the call back to bind() here, which
    // fills them with the bridge's.
    public class Bridge {
        private static var manager:Manager;

        public static function start():void {
            manager = new Manager();
            ExternalInterface.addCallback("elitegfx.bind", bind);
            ExternalInterface.addCallback("HandleMessage", manager.HandleMessage);
            // Registered last: the game takes it as "the bridge is up".
            ExternalInterface.addCallback("elitegfx.store", Store.set);
        }

        private static function bind(frameRate:Number):void {
            Tweener.start(frameRate);
            var managers:Object = lookup("core.managers.CommunicationManagerFactory");
            if (managers != null) {
                managers.instance.getCommunicationManagerObject = function():Object {
                    return manager;
                };
            }
            var tweeners:Object = lookup("core.tweening.TweenerFactory");
            if (tweeners != null) {
                tweeners.instance.getTweenerObject = function():Object {
                    return new Tweener();
                };
            }
            var store:Object = lookup("core.managers.DataStore");
            if (store != null) {
                Store.install(store.instance);
            }
            // No tutorial: the interface is bound (its functions may be called) and never in one.
            var tutorial:Object = lookup("elite.tutorial.EliteTutorialManager");
            if (tutorial != null) {
                var t:Object = tutorial.instance;
                t.register = function(...rest):void {};
                t.unregister = function(...rest):void {};
                t.bound = true;
                t.inTutorial = false;
            }
        }

        // A class of the movie's, or null if this movie has none of that name.
        private static function lookup(name:String):Object {
            try {
                return getDefinitionByName(name);
            } catch (error:ReferenceError) {
            }
            return null;
        }
    }
}

import elitegfx.Bridge;

Bridge.start();
