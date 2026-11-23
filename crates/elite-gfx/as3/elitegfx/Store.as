package elitegfx {
    // The DataStore Elite's exe gives the movies (core.managers.DataStore's seven function slots,
    // ui_messages_panels.md 0): a tree of nodes, each named by its context, an array path, with
    // properties. The game writes it (Movie::store); the movies read it and listen to it.
    //
    // What the movies' code expects (core/dataStructures/DataStoreCollection.as, core/util/DataHelper.as,
    // the panels' models): a property listener is NOT called when it is added (the code reads the value
    // itself with getValue first, or DataHelper.forceGetValueAll); a children listener IS told the
    // current children at once ("Complete" with the array of child contexts), because
    // DataStoreCollection starts empty and never asks for them otherwise. A listener is an object the
    // code adds its own fields to, with release(). Stand-ins: children are ordered by the listener's
    // sort field (numbers ascending, nodes without it last, in the order they were made), and a
    // change of a sort value is told as the whole new order ("Complete"); listener priorities are not
    // used; nodes are never removed (Movie::store only writes).
    public class Store {
        private static const SEP:String = "\u001f";
        private static const ADDED:String = "Added";
        private static const COMPLETE:String = "Complete";

        // node key -> {context:Array, props:Object, children:Array of child keys}
        private static var nodes:Object = {};
        // node key -> {property -> [listener]}
        private static var propertyListeners:Object = {};
        // node key -> [listener]
        private static var childListeners:Object = {};

        // Fills core.managers.DataStore.instance's slots (the movie's own stand-in class).
        public static function install(store:Object):void {
            store.getPropertyListener = getPropertyListener;
            store.getContextChildrenListener = getContextChildrenListener;
            store.getValue = getValue;
            if (store.hasOwnProperty("getChildrenOfContext")) store.getChildrenOfContext = getChildrenOfContext;
            if (store.hasOwnProperty("contextHasChild")) store.contextHasChild = contextHasChild;
            if (store.hasOwnProperty("doesContextExist")) store.doesContextExist = doesContextExist;
            if (store.hasOwnProperty("getNearestExistingAncestor")) store.getNearestExistingAncestor = getNearestExistingAncestor;
        }

        // From the game: writes one property, making the node and its parents if they are new.
        // A new node is made with the property already in it, so it is announced at its sorted place.
        // Listeners hear a write only when it changes the value; a change of a value children are
        // sorted by is announced to those children's listeners as a new order ("Complete").
        public static function set(context:Array, property:String, value:*):void {
            var k:String = key(context);
            var node:Object = nodes[k];
            if (node == null) {
                var props:Object = {};
                props[property] = value;
                make(context, props);
            } else {
                if (property in node.props && same(node.props[property], value)) {
                    return;
                }
                node.props[property] = value;
                if (context.length > 0) {
                    resort(context.slice(0, context.length - 1), property);
                }
            }
            var listeners:Object = propertyListeners[k];
            var list:Array = listeners == null ? null : listeners[property];
            if (list != null) {
                for each (var l:Object in list.concat()) {
                    l.callback(value);
                }
            }
        }

        // Whether two values written by the game are equal: primitives by value, arrays and plain
        // objects (what messages carry) field by field.
        private static function same(a:*, b:*):Boolean {
            if (a === b) return true;
            if (a is Array && b is Array) {
                if (a.length != b.length) return false;
                for (var i:int = 0; i < a.length; i++) {
                    if (!same(a[i], b[i])) return false;
                }
                return true;
            }
            if (a == null || b == null || typeof a != "object" || typeof b != "object" || a is Array || b is Array) {
                return false;
            }
            var n:String;
            for (n in a) {
                if (!(n in b) || !same(a[n], b[n])) return false;
            }
            for (n in b) {
                if (!(n in a)) return false;
            }
            return true;
        }

        // Tells the children listeners of `parentContext` that sort by `property` the new order.
        private static function resort(parentContext:Array, property:String):void {
            var parent:Object = nodes[key(parentContext)];
            var list:Array = childListeners[key(parentContext)];
            if (parent == null || list == null) return;
            for each (var l:Object in list.concat()) {
                if (l.sortField == property) {
                    l.callback(COMPLETE, contexts(parent, l.sortField), -1);
                }
            }
        }

        private static function key(context:Array):String {
            return context == null ? "" : context.join(SEP);
        }

        private static function copy(context:Array):Array {
            var out:Array = [];
            for each (var part:* in context) out.push(String(part));
            return out;
        }

        // The node of `context`, made (with `props`, and its parents) and announced if it is new.
        private static function make(context:Array, props:Object = null):Object {
            var k:String = key(context);
            var node:Object = nodes[k];
            if (node != null) {
                return node;
            }
            node = {context: copy(context), props: props == null ? {} : props, children: []};
            nodes[k] = node;
            if (context.length > 0) {
                var parentContext:Array = context.slice(0, context.length - 1);
                var parent:Object = make(parentContext);
                parent.children.push(k);
                var list:Array = childListeners[key(parentContext)];
                if (list != null) {
                    for each (var l:Object in list.concat()) {
                        l.callback(ADDED, node.context.concat(), sorted(parent, l.sortField).indexOf(k));
                    }
                }
            }
            return node;
        }

        // A node's child keys in the order a listener with that sort field sees them.
        private static function sorted(node:Object, sortField:String):Array {
            var keys:Array = node.children.concat();
            if (sortField) {
                var order:Function = function(k:String):Number {
                    var v:* = nodes[k].props[sortField];
                    return v === undefined || v === null || isNaN(Number(v)) ? Number.POSITIVE_INFINITY : Number(v);
                };
                var made:Array = node.children;
                keys.sort(function(a:String, b:String):int {
                    var d:Number = order(a) - order(b);
                    if (d < 0) return -1;
                    if (d > 0) return 1;
                    return made.indexOf(a) - made.indexOf(b);
                });
            }
            return keys;
        }

        private static function contexts(node:Object, sortField:String):Array {
            var out:Array = [];
            for each (var k:String in sorted(node, sortField)) out.push(nodes[k].context.concat());
            return out;
        }

        private static function listener(table:Object, k:String, callback:Function):Object {
            var l:Object = {callback: callback};
            l.release = function():void {
                var list:Array = table[k];
                if (list != null && list.indexOf(l) >= 0) list.splice(list.indexOf(l), 1);
            };
            return l;
        }

        private static function getPropertyListener(context:Array, property:String, callback:Function, priority:Number = 0):Object {
            var k:String = key(context);
            var byProperty:Object = propertyListeners[k];
            if (byProperty == null) {
                byProperty = {};
                propertyListeners[k] = byProperty;
            }
            if (byProperty[property] == null) byProperty[property] = [];
            var l:Object = listener(byProperty, property, callback);
            byProperty[property].push(l);
            return l;
        }

        private static function getContextChildrenListener(context:Array, callback:Function, sortField:String = null, priority:Number = 0):Object {
            var k:String = key(context);
            if (childListeners[k] == null) childListeners[k] = [];
            var l:Object = listener(childListeners, k, callback);
            l.sortField = sortField;
            childListeners[k].push(l);
            var node:Object = nodes[k];
            callback(COMPLETE, node == null ? [] : contexts(node, sortField), -1);
            return l;
        }

        private static function getValue(context:Array, property:String):* {
            var node:Object = nodes[key(context)];
            return node == null ? undefined : node.props[property];
        }

        private static function getChildrenOfContext(context:Array, ...rest):Array {
            var node:Object = nodes[key(context)];
            return node == null ? [] : contexts(node, rest.length > 0 ? rest[0] : null);
        }

        private static function contextHasChild(context:Array, child:String):Boolean {
            return nodes[key(context.concat([child]))] != null;
        }

        private static function doesContextExist(context:Array):Boolean {
            return nodes[key(context)] != null;
        }

        private static function getNearestExistingAncestor(context:Array):Array {
            for (var n:int = context.length; n > 0; n--) {
                var part:Array = context.slice(0, n);
                if (nodes[key(part)] != null) return copy(part);
            }
            return [];
        }
    }
}
