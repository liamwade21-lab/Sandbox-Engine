package elitegfxproof {
    import elitegfx.Store;
    import elitegfx.Tweener;
    import flash.external.ExternalInterface;

    // A code block for elite-gfx's own test (src/tests/bridge_proof.rs), never put in a movie the game
    // runs: it drives the bridge's tweener and DataStore and traces what they do.
    public class Proof {
        public static function start():void {
            // As a movie does: the game hands this to the bridge, which starts the tweener's clock.
            ExternalInterface.call("BindDirectInterfaces");
            var tweener:Tweener = new Tweener();
            var good:Object = {x: 0};
            tweener.addTween(null, 0.1, {alpha: 1});
            tweener.addTween(new Throwing(), 0.1, {x: 1});
            tweener.addTween(good, 0.2, {x: 10}, 0, 0, function():void {
                say("good " + good.x);
            });

            var store:Object = {getChildrenOfContext: null, contextHasChild: null, doesContextExist: null,
                                getNearestExistingAncestor: null};
            Store.install(store);
            store.getContextChildrenListener(["r"], function(kind:String, child:Object, index:int):void {
                say("children " + kind + " " + describe(child) + " " + index);
            }, "s");
            store.getPropertyListener(["r", "a"], "v", function(value:*):void {
                say("a.v " + (value is Object && !(value is Number) ? value.x : value));
            });
        }

        private static function describe(child:Object):String {
            if (child is Array && child.length > 0 && child[0] is Array) {
                var parts:Array = [];
                for each (var c:Array in child) parts.push(c.join("/"));
                return parts.join(",");
            }
            return (child as Array).join("/");
        }

        private static function say(text:String):void {
            ExternalInterface.call("Trace", "proof: " + text);
        }
    }
}

class Throwing {
    public function get x():Number {
        return 0;
    }

    public function set x(value:Number):void {
        throw new Error("setter throws");
    }
}

import elitegfxproof.Proof;

Proof.start();
